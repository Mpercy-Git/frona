import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// The bus only needs a token and a fetch; both are stubbed so the reconnect
// loop can be driven deterministically.
vi.mock("../api-client", () => ({
  API_URL: "",
  ensureAccessToken: async () => ({ ok: false as const }),
  refreshStaleToken: async () => ({ ok: false as const }),
}));

import { SSEEventBus, type ChatSSEEvent } from "../sse-event-bus";

/** A response whose body ends immediately (the stream the server dropped). */
function closedStream() {
  return {
    ok: true,
    status: 200,
    body: { getReader: () => ({ read: () => Promise.resolve({ done: true, value: undefined }) }) },
  };
}

/** A response whose body never yields — an open, healthy stream. */
function openStream() {
  return {
    ok: true,
    status: 200,
    body: { getReader: () => ({ read: () => new Promise<never>(() => {}) }) },
  };
}

describe("SSEEventBus: a chat subscription survives a stream reconnect", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("keeps delivering events to a chat subscribed before the drop", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(closedStream())
      .mockResolvedValue(openStream());
    vi.stubGlobal("fetch", fetchMock);

    const bus = new SSEEventBus();
    const controller = new AbortController();

    // Consume exactly the way useChatRuntime does.
    const received: ChatSSEEvent[] = [];
    void (async () => {
      for await (const event of bus.subscribe("chat-1", controller.signal)) {
        received.push(event);
      }
    })();
    await vi.advanceTimersByTimeAsync(0);

    const reconnected = vi.fn();
    bus.onReconnect(reconnected);

    bus.connect(controller.signal);
    // First stream ends, backoff elapses, second stream connects: a reconnect.
    await vi.advanceTimersByTimeAsync(1100);
    expect(reconnected).toHaveBeenCalledTimes(1);

    // The agent finishes on the new stream.
    bus.routeEvent("inference_done", "chat-1", {
      message: { id: "msg-1", chat_id: "chat-1", role: "agent", content: "done", status: "completed", created_at: "" },
    });
    await vi.advanceTimersByTimeAsync(0);

    expect(received.map((e) => e.type)).toEqual(["inference_done"]);

    controller.abort();
  });

  it("drops events buffered by the dropped stream", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(closedStream())
      .mockResolvedValue(openStream());
    vi.stubGlobal("fetch", fetchMock);

    const bus = new SSEEventBus();
    const controller = new AbortController();

    const received: ChatSSEEvent[] = [];
    const iterable = bus.subscribe("chat-1", controller.signal);
    const iterator = iterable[Symbol.asyncIterator]();
    // Queue an event with no read pending, so it lands in the subscriber's
    // buffer rather than resolving a waiting read.
    bus.routeEvent("token", "chat-1", { content: "stale" });

    bus.connect(controller.signal);
    await vi.advanceTimersByTimeAsync(1100);

    bus.routeEvent("token", "chat-1", { content: "fresh" });
    const next = await iterator.next();
    if (!next.done) received.push(next.value);

    expect(received).toEqual([{ type: "token", content: "fresh" }]);

    controller.abort();
  });
});
