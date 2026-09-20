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
