import { describe, it, expect, vi, beforeEach } from "vitest";

// `loadMessages` is the only api-client dependency these tests exercise; keep
// the rest of the module real.
vi.mock("../api-client", async (importActual) => {
  const actual = await importActual<typeof import("../api-client")>();
  return { ...actual, api: { ...actual.api, get: vi.fn() } };
});

import { ChatStore } from "../chat-store";
import { api } from "../api-client";
import type { MessageResponse } from "../types";

const mockGet = api.get as unknown as ReturnType<typeof vi.fn>;

function agentMessage(overrides: Partial<MessageResponse> = {}): MessageResponse {
  return {
    id: "msg-1",
    chat_id: "chat-1",
    role: "agent",
    content: "working on it",
    status: "completed",
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

/** Answer the history fetch with `messages`; fail any usage seed harmlessly. */
function serve(messages: MessageResponse[]) {
  mockGet.mockImplementation((path: string) => {
    if (path.includes("/messages")) {
      return Promise.resolve({ messages, has_more: false });
    }
    return Promise.reject(new Error("no usage seed in this test"));
  });
}

beforeEach(() => {
  mockGet.mockReset();
});

describe("ChatStore.loadMessages", () => {
  // Opening a chat mid-run (a reload, or a task part-way through its turns)
  // used to leave isRunning false, so the composer offered Send instead of
  // Stop — with nothing to change that until the next SSE event arrived,
  // which can be minutes away while one long tool call runs.
  it("shows the thread as running when the last message is still executing", async () => {
    serve([
      agentMessage({ id: "msg-old" }),
      agentMessage({ id: "msg-live", status: "executing" }),
    ]);

    const store = new ChatStore();
    await store.loadMessages("chat-1");

    expect(store.getSnapshot().isRunning).toBe(true);
  });

  it("stays idle when the last message has finished", async () => {
    serve([agentMessage({ id: "msg-live", status: "completed" })]);

    const store = new ChatStore();
    await store.loadMessages("chat-1");

    expect(store.getSnapshot().isRunning).toBe(false);
  });

  // A cancelled turn is finished: reopening the chat must not resurrect the
  // spinner for a run the user already stopped.
  it("stays idle when the last message was cancelled", async () => {
    serve([
      agentMessage({ id: "msg-live", status: "cancelled", content: "partial" }),
    ]);

    const store = new ChatStore();
    await store.loadMessages("chat-1");

    expect(store.getSnapshot().isRunning).toBe(false);
  });
});

// The reconnect path reloads history on a store that is still showing the
// dropped stream as running. When the reload says the turn already finished,
// the thread has to go back to idle: otherwise the spinner and the Stop
// button outlive the run that completed while the stream was down, and only
// a page reload (or pressing Stop) clears them.
describe("ChatStore.loadMessages after a dropped stream", () => {
  it("returns to idle when the reload shows the run already finished", async () => {
    serve([agentMessage({ id: "msg-live", status: "completed", content: "all done" })]);

    const store = new ChatStore();
    store.handleEvent({ type: "token", content: "all do" });
    expect(store.isRunning).toBe(true);

    await store.loadMessages("chat-1");

    expect(store.getSnapshot().isRunning).toBe(false);
    expect(store.streamingText).toBe("");
  });

  it("keeps the spinner while a just-sent message has no server echo yet", async () => {
    serve([agentMessage({ id: "msg-old", status: "completed" })]);

    const store = new ChatStore();
    store.addUserMessage("and one more thing");

    await store.loadMessages("chat-1");

    expect(store.getSnapshot().isRunning).toBe(true);
  });
});

// A dropped stream reloads history mid-send. The optimistic placeholder and
// the persisted row are the same message, but they carry different ids, so
// id-based dedupe cannot pair them: the merge kept both, and the echo that
// followed overwrote the placeholder with a second copy of the row. Either
// way the user sees their own message twice.
describe("ChatStore optimistic message reconciliation", () => {
  function userMessage(overrides: Partial<MessageResponse> = {}): MessageResponse {
    return {
      id: "msg-real",
      chat_id: "chat-1",
      role: "user",
      content: "second message",
      created_at: "2026-01-01T00:01:00Z",
      ...overrides,
    };
  }

  const userBubbles = (store: ChatStore) =>
    store.getDisplayMessages().filter((m) => m.role === "user");

  it("does not double a sent message when a reload races the echo", async () => {
    serve([agentMessage({ id: "msg-first" }), userMessage()]);

    const store = new ChatStore();
    store.addUserMessage("second message");

    // The reconnect handler's catch-up fetch: the server already persisted it.
    await store.loadMessages("chat-1");

    expect(userBubbles(store)).toHaveLength(1);

    // The echo then arrives on the reconnected stream.
    store.handleEvent({ type: "chat_message", message: userMessage() });

    expect(userBubbles(store)).toHaveLength(1);
    expect(userBubbles(store)[0].id).toBe("msg-real");
  });

  it("keeps a placeholder the history has not caught up with", async () => {
    serve([agentMessage({ id: "msg-first" })]);

    const store = new ChatStore();
    store.addUserMessage("second message");

    await store.loadMessages("chat-1");

    expect(userBubbles(store)).toHaveLength(1);
    expect(userBubbles(store)[0].id).toMatch(/^__user_/);

    store.handleEvent({ type: "chat_message", message: userMessage() });

    expect(userBubbles(store)).toHaveLength(1);
    expect(userBubbles(store)[0].id).toBe("msg-real");
  });

  it("does not double an echo that repeats a message already in the thread", () => {
    const store = new ChatStore();
    store.handleEvent({ type: "chat_message", message: userMessage() });
    store.addUserMessage("a third message");
    // A replayed echo of the first message, with the second still pending.
    store.handleEvent({ type: "chat_message", message: userMessage() });

    expect(userBubbles(store).map((m) => m.content)).toEqual([
      "second message",
      "a third message",
    ]);
  });

  // Pairing a placeholder to a persisted row on content alone treats the text
  // as a key. It is not one: people repeat themselves — "ok", "yes", "go on" —
  // and the second one then reconciles against the first, so the message the
  // user just sent vanishes from the thread until its own echo arrives.
  describe("when the same text is sent twice", () => {
    const earlier = (): MessageResponse =>
      userMessage({ id: "msg-ok-1", content: "ok", created_at: "2026-01-01T00:00:30Z" });

    it("keeps the pending message when a reload replays the earlier one", async () => {
      serve([earlier()]);

      const store = new ChatStore();
      await store.loadMessages("chat-1");
      // The user says "ok" a second time; the reconnect's catch-up fetch runs
      // before the server has persisted it.
      store.addUserMessage("ok");
      await store.loadMessages("chat-1");

      expect(userBubbles(store).map((m) => m.content)).toEqual(["ok", "ok"]);
      expect(userBubbles(store)[1].id).toMatch(/^__user_/);
    });

    it("pairs the placeholder with its own row, not the earlier twin", async () => {
      const store = new ChatStore();
      serve([earlier()]);
      await store.loadMessages("chat-1");
      store.addUserMessage("ok");

      // The reload now carries both: the earlier row and the one just stored.
      serve([earlier(), userMessage({ id: "msg-ok-2", content: "ok" })]);
      await store.loadMessages("chat-1");

      expect(userBubbles(store).map((m) => m.id)).toEqual(["msg-ok-1", "msg-ok-2"]);
    });

    it("keeps the pending message when the earlier one is re-broadcast", () => {
      const store = new ChatStore();
      store.handleEvent({ type: "chat_message", message: earlier() });
      store.addUserMessage("ok");
      // The server re-sends the earlier row — an update, or a reconnect replay.
      store.handleEvent({ type: "chat_message", message: earlier() });

      expect(userBubbles(store).map((m) => m.content)).toEqual(["ok", "ok"]);
      expect(userBubbles(store)[1].id).toMatch(/^__user_/);

      // The pending message's own echo still lands on its placeholder.
      store.handleEvent({
        type: "chat_message",
        message: userMessage({ id: "msg-ok-2", content: "ok" }),
      });

      expect(userBubbles(store).map((m) => m.id)).toEqual(["msg-ok-1", "msg-ok-2"]);
    });
  });
});
