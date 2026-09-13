import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

const apiClient = vi.hoisted(() => ({ getChatActivity: vi.fn() }));
vi.mock("@/lib/api-client", () => apiClient);

import { sseBus } from "../sse-event-bus";
import {
  ChatActivityProvider,
  useChatActivity,
  useAggregateActivity,
} from "../chat-activity-context";

function Probe({ chatId }: { chatId: string }) {
  const { activityOf, workingCount, waitingCount } = useChatActivity();
  return (
    <div>
      <span data-testid="activity">{activityOf(chatId)}</span>
      <span data-testid="counts">{`${workingCount}/${waitingCount}`}</span>
    </div>
  );
}

function renderProbe(chatId = "chat-1") {
  return render(
    <ChatActivityProvider>
      <Probe chatId={chatId} />
    </ChatActivityProvider>,
  );
}

const activity = () => screen.getByTestId("activity").textContent;
const counts = () => screen.getByTestId("counts").textContent;

/** Push an event through the real bus, the way the stream would. */
function emit(chatId: string, kind: string, payload: Record<string, unknown> = {}) {
  act(() => {
    sseBus.routeEvent(kind, chatId, payload);
  });
}

describe("ChatActivityProvider", () => {
  beforeEach(() => {
    apiClient.getChatActivity.mockReset();
    apiClient.getChatActivity.mockResolvedValue({ working: [], waiting: [] });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("seeds from the snapshot, so a turn that started before this client connected still shows", async () => {
    apiClient.getChatActivity.mockResolvedValue({ working: ["chat-1"], waiting: [] });
    renderProbe();

    await waitFor(() => expect(activity()).toBe("working"));
    expect(counts()).toBe("1/0");
  });

  it("marks a chat waiting when the snapshot says it is parked on a human", async () => {
    apiClient.getChatActivity.mockResolvedValue({ working: [], waiting: ["chat-1"] });
    renderProbe();

    await waitFor(() => expect(activity()).toBe("waiting"));
    expect(counts()).toBe("0/1");
  });

  it("follows the stream through a whole turn", async () => {
    renderProbe();
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());

    emit("chat-1", "inference_start");
    expect(activity()).toBe("working");

    emit("chat-1", "inference_paused", { reason: { type: "Hitl" }, message: { id: "m1" } });
    expect(activity()).toBe("waiting");

    emit("chat-1", "inference_resume", { message: { id: "m1" } });
    expect(activity()).toBe("working");

    emit("chat-1", "inference_done", { message: { id: "m1" } });
    expect(activity()).toBe("idle");
    expect(counts()).toBe("0/0");
  });

  it("keeps activity per chat", async () => {
    renderProbe("chat-1");
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());

    emit("chat-2", "inference_start");

    expect(activity()).toBe("idle");
    expect(counts()).toBe("1/0");
  });

  it("re-reads the snapshot when the stream reconnects", async () => {
    renderProbe();
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalledTimes(1));

    // A dropped stream can swallow the terminal event, so the snapshot is the
    // only way back to the truth.
    emit("chat-1", "inference_start");
    expect(activity()).toBe("working");

    apiClient.getChatActivity.mockResolvedValue({ working: [], waiting: [] });
    await act(async () => {
      sseBus.handleReconnect();
    });

    await waitFor(() => expect(activity()).toBe("idle"));
  });

  it("keeps an event that lands while the snapshot request is in flight", async () => {
    let resolveSnapshot: (value: { working: string[]; waiting: string[] }) => void = () => {};
    apiClient.getChatActivity.mockReturnValue(
      new Promise<{ working: string[]; waiting: string[] }>((r) => {
        resolveSnapshot = r;
      }),
    );

    renderProbe();
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());

    emit("chat-1", "inference_start");
    await act(async () => {
      resolveSnapshot({ working: [], waiting: [] });
    });

    // The snapshot was taken before the turn started; the newer event wins.
    await waitFor(() => expect(activity()).toBe("working"));
  });

  it("sweeps a working chat that never reported finishing", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderProbe();
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());

    emit("chat-1", "inference_start");
    expect(activity()).toBe("working");

    // A server restart never sends the terminal event; a row must not spin
    // for ever because of it.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(16 * 60 * 1000);
    });

    expect(activity()).toBe("idle");
  });

  it("does not sweep a chat waiting on a human, however long they take", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderProbe();
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());

    emit("chat-1", "inference_paused", { reason: { type: "Hitl" }, message: { id: "m1" } });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60 * 60 * 1000);
    });

    expect(activity()).toBe("waiting");
  });

  it("survives a snapshot that fails to load", async () => {
    apiClient.getChatActivity.mockRejectedValue(new Error("offline"));
    renderProbe();

    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());
    expect(activity()).toBe("idle");

    // The stream still drives the indicators.
    emit("chat-1", "inference_start");
    expect(activity()).toBe("working");
  });
});

describe("ChatActivityProvider: the open conversation", () => {
  beforeEach(() => {
    apiClient.getChatActivity.mockReset();
    apiClient.getChatActivity.mockResolvedValue({ working: [], waiting: [] });
  });

  function LocalProbe({ chatId, activity }: { chatId: string; activity: "working" | "idle" }) {
    const { setLocalActivity, clearLocalActivity } = useChatActivity();
    return (
      <div>
        <button onClick={() => setLocalActivity(chatId, activity)}>set</button>
        <button onClick={() => clearLocalActivity(chatId)}>clear</button>
      </div>
    );
  }

  it("lets the mounted chat report sooner than the stream does", async () => {
    render(
      <ChatActivityProvider>
        <Probe chatId="chat-1" />
        <LocalProbe chatId="chat-1" activity="working" />
      </ChatActivityProvider>,
    );
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());

    act(() => {
      screen.getByText("set").click();
    });

    expect(activity()).toBe("working");
    expect(counts()).toBe("1/0");
  });

  it("leaves a backgrounded chat running when its view unmounts", async () => {
    render(
      <ChatActivityProvider>
        <Probe chatId="chat-1" />
        <LocalProbe chatId="chat-1" activity="idle" />
      </ChatActivityProvider>,
    );
    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());

    emit("chat-1", "inference_start");
    // While mounted, the chat's own store is authoritative.
    act(() => {
      screen.getByText("set").click();
    });
    expect(activity()).toBe("idle");

    // Unmounting drops the override rather than declaring the chat idle — the
    // turn is still running in the background.
    act(() => {
      screen.getByText("clear").click();
    });
    expect(activity()).toBe("working");
  });
});

describe("useAggregateActivity", () => {
  beforeEach(() => {
    apiClient.getChatActivity.mockReset();
    apiClient.getChatActivity.mockResolvedValue({ working: [], waiting: [] });
  });

  function Aggregate({ chatIds }: { chatIds: string[] }) {
    return <span data-testid="activity">{useAggregateActivity(chatIds)}</span>;
  }

  function renderAggregate(chatIds: string[]) {
    return render(
      <ChatActivityProvider>
        <Aggregate chatIds={chatIds} />
      </ChatActivityProvider>,
    );
  }

  it("reports a group as working when any member is", async () => {
    apiClient.getChatActivity.mockResolvedValue({ working: ["chat-2"], waiting: [] });
    renderAggregate(["chat-1", "chat-2"]);

    await waitFor(() => expect(activity()).toBe("working"));
  });

  it("prefers working over waiting — it is the state still changing", async () => {
    apiClient.getChatActivity.mockResolvedValue({ working: ["chat-2"], waiting: ["chat-1"] });
    renderAggregate(["chat-1", "chat-2"]);

    await waitFor(() => expect(activity()).toBe("working"));
  });

  it("is idle when nothing in the group is active", async () => {
    apiClient.getChatActivity.mockResolvedValue({ working: ["chat-9"], waiting: [] });
    renderAggregate(["chat-1", "chat-2"]);

    await waitFor(() => expect(apiClient.getChatActivity).toHaveBeenCalled());
    expect(activity()).toBe("idle");
  });
});
