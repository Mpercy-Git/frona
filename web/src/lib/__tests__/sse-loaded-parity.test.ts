import { describe, it, expect } from "vitest";
import { ChatStore } from "../chat-store";
import { convertMessage } from "../use-chat-runtime";
import type { MessageResponse, ToolCall } from "../types";

/**
 * These tests verify the critical invariant:
 *
 *   SSE events streamed through ChatStore → final messages
 *   MUST produce the same converted output as
 *   loading those same messages from the API.
 *
 * This ensures components render identically whether the user watched
 * the stream live or loaded the conversation from history.
 */

function makeToolCall(overrides: Partial<ToolCall> = {}): ToolCall {
  return {
    id: "te-1",
    chat_id: "chat-1",
    message_id: "msg-1",
    turn: 1,
    provider_call_id: "tc-1",
    name: "web_search",
    arguments: { query: "test" },
    result: "Found results",
    success: true,
    duration_ms: 100,
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

/**
 * Normalize converted message for comparison:
 * - Strip volatile fields (createdAt)
 * - Sort content parts deterministically
 */
function normalize(converted: ReturnType<typeof convertMessage>) {
  if (!converted) return null;
  const { createdAt, ...rest } = converted as any;
  return rest;
}

// ---------------------------------------------------------------------------
// Parity: simple text message
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: simple text", () => {
  it("produces identical converted output for a plain text agent response", () => {
    // --- SSE path ---
    const sseStore = new ChatStore();

    sseStore.handleEvent({
      type: "chat_message",
      message: {
        id: "user-1",
        chat_id: "chat-1",
        role: "user",
        content: "Hello",
        created_at: "2026-01-01T00:00:00Z",
      },
    });
    sseStore.handleEvent({ type: "token", content: "Hi there!" });

    const agentMsg: MessageResponse = {
      id: "agent-1",
      chat_id: "chat-1",
      role: "agent",
      content: "Hi there!",
      agent_id: "system",
      status: "completed",
      created_at: "2026-01-01T00:00:01Z",
    };
    sseStore.handleEvent({ type: "inference_done", message: agentMsg });

    const sseMessages = sseStore.getDisplayMessages();
    const sseConverted = sseMessages.map((m) => normalize(convertMessage(m)));

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [
      {
        id: "user-1",
        chat_id: "chat-1",
        role: "user",
        content: "Hello",
        created_at: "2026-01-01T00:00:00Z",
      },
      agentMsg,
    ];
    loadedStore.loaded = true;

    const loadedMessages = loadedStore.getDisplayMessages();
    const loadedConverted = loadedMessages.map((m) => normalize(convertMessage(m)));

    expect(sseConverted).toEqual(loadedConverted);
  });
});

// ---------------------------------------------------------------------------
// Parity: text + tool calls
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: text with tool calls", () => {
  it("produces identical output for a message with tool executions", () => {
    const toolExecution = makeToolCall({
      id: "tc-1",
      provider_call_id: "tc-1",
      name: "web_search",
      arguments: { query: "cats" },
      result: "10 results found",
      description: "Searching for cats",
      turn_text: "Let me search for cats.",
    });

    const agentMsg: MessageResponse = {
      id: "agent-1",
      chat_id: "chat-1",
      role: "agent",
      content: "",
      agent_id: "system",
      status: "completed",
      tool_calls: [toolExecution],
      created_at: "2026-01-01T00:00:01Z",
    };

    // --- SSE path ---
    const sseStore = new ChatStore();
    sseStore.handleEvent({ type: "token", content: "Let me search for cats." });
    sseStore.handleEvent({
      type: "tool_call",
      id: "te-1",
      provider_call_id: "tc-1",
      name: "web_search",
      arguments: '{"query":"cats"}',
      description: "Searching for cats",
    });
    sseStore.handleEvent({
      type: "tool_result",
      name: "web_search",
      success: true,
      summary: "10 results found",
    });
    sseStore.handleEvent({ type: "inference_done", message: agentMsg });

    const sseMessages = sseStore.getDisplayMessages();
    const sseConverted = sseMessages.map((m) => normalize(convertMessage(m)));

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [agentMsg];
    loadedStore.loaded = true;

    const loadedMessages = loadedStore.getDisplayMessages();
    const loadedConverted = loadedMessages.map((m) => normalize(convertMessage(m)));

    expect(sseConverted).toEqual(loadedConverted);
  });
});

// ---------------------------------------------------------------------------
// Parity: multiple tool calls
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: multiple tool calls", () => {
  it("produces identical output for sequential tool calls", () => {
    const te1 = makeToolCall({
      id: "tc-1",
      provider_call_id: "tc-1",
      name: "web_search",
      arguments: { query: "topic" },
      result: "Found info",
      description: "Searching",
    });
    const te2 = makeToolCall({
      id: "tc-2",
      provider_call_id: "tc-2",
      name: "cli",
      arguments: { command: "echo done" },
      result: "done\n",
      description: "Running command",
    });

    const agentMsg: MessageResponse = {
      id: "agent-1",
      chat_id: "chat-1",
      role: "agent",
      content: "Here is what I found.",
      agent_id: "system",
      status: "completed",
      tool_calls: [te1, te2],
      created_at: "2026-01-01T00:00:01Z",
    };

    // --- SSE path ---
    const sseStore = new ChatStore();
    sseStore.handleEvent({ type: "tool_call", id: "te-1", provider_call_id: "tc-1", name: "web_search", arguments: '{"query":"topic"}', description: "Searching" });
    sseStore.handleEvent({ type: "tool_result", name: "web_search", success: true, summary: "Found info" });
    sseStore.handleEvent({ type: "tool_call", id: "te-2", provider_call_id: "tc-2", name: "cli", arguments: '{"command":"echo done"}', description: "Running command" });
    sseStore.handleEvent({ type: "tool_result", name: "cli", success: true, summary: "done\n" });
    sseStore.handleEvent({ type: "token", content: "Here is what I found." });
    sseStore.handleEvent({ type: "inference_done", message: agentMsg });

    const sseConverted = sseStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [agentMsg];
    loadedStore.loaded = true;

    const loadedConverted = loadedStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    expect(sseConverted).toEqual(loadedConverted);
  });
});

// ---------------------------------------------------------------------------
// Parity: external tool (inference_paused)
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: external tool (Question)", () => {
  it("produces identical output for a pending external tool", () => {
    const hitl = {
      prompt: "Which option?",
      url: "/chats/c1",
      request: { type: "Question" as const, data: { options: ["A", "B"] } },
      status: "pending" as const,
      response: null,
      delivery: null,
    };

    const externalTe = makeToolCall({
      id: "te-ext",
      provider_call_id: "tc-ext",
      name: "ask_user_question",
      message_id: "msg-ext",
      hitl,
    });

    // The message as it would be stored/loaded from the API
    const storedMsg: MessageResponse = {
      id: "msg-ext",
      chat_id: "chat-1",
      role: "agent",
      content: "Let me ask you something.",
      agent_id: "system",
      status: "executing",
      tool_calls: [externalTe],
      created_at: "2026-01-01T00:00:01Z",
    };

    // --- SSE path ---
    const sseStore = new ChatStore();
    sseStore.handleEvent({ type: "token", content: "Let me ask you something." });
    sseStore.handleEvent({
      type: "tool_call",
      id: "te-ext",
      provider_call_id: "tc-ext",
      name: "ask_user_question",
      arguments: '{"question":"Which option?"}',
    });
    sseStore.handleEvent({ type: "inference_paused", reason: { type: "Hitl" }, message: storedMsg });

    const sseMessages = sseStore.getDisplayMessages();

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [storedMsg];
    loadedStore.loaded = true;

    const loadedMessages = loadedStore.getDisplayMessages();

    // Both should have the same hitl on the external tool execution
    const sseTool = sseMessages[0]?.tool_calls?.find((t) => t.id === "te-ext");
    const loadedTool = loadedMessages[0]?.tool_calls?.find((t) => t.id === "te-ext");
    expect(sseTool?.hitl).toEqual(loadedTool?.hitl);

    // Status should match
    expect(sseMessages[0]?.status).toBe(loadedMessages[0]?.status);
  });
});

// ---------------------------------------------------------------------------
// Parity: resolved external tool
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: resolved external tool", () => {
  it("produces identical output after inference_resume", () => {
    const resolvedToolData = {
      type: "Question" as const,
      data: {
        question: "Pick one",
        options: ["A", "B"],
        status: "resolved" as const,
        response: "A",
      },
    };

    const resolvedTe = makeToolCall({
      id: "te-q1",
      provider_call_id: "tc-q1",
      name: "ask_user_question",
      tool_data: resolvedToolData,
      result: "A",
    });

    const resolvedMsg: MessageResponse = {
      id: "msg-r1",
      chat_id: "chat-1",
      role: "agent",
      content: "",
      agent_id: "system",
      status: "completed",
      tool_calls: [resolvedTe],
      created_at: "2026-01-01T00:00:01Z",
    };

    // --- SSE path: tool was pending, then resolved ---
    const sseStore = new ChatStore();
    // Start with the pending message
    sseStore.messages.push({
      id: "msg-r1",
      chat_id: "chat-1",
      role: "agent",
      content: "",
      agent_id: "system",
      status: "executing",
      tool_calls: [
        makeToolCall({
          id: "te-q1",
          provider_call_id: "tc-q1",
          tool_data: {
            type: "Question",
            data: { question: "Pick one", options: ["A", "B"], status: "pending", response: null },
          },
        }),
      ],
      created_at: "2026-01-01T00:00:01Z",
    });

    // Resolve via inference_resume
    sseStore.handleEvent({ type: "inference_resume", message: resolvedMsg });

    const sseConverted = sseStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [resolvedMsg];
    loadedStore.loaded = true;

    const loadedConverted = loadedStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    expect(sseConverted).toEqual(loadedConverted);
  });
});

// ---------------------------------------------------------------------------
// Parity: multi-turn conversation
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: multi-turn conversation", () => {
  it("handles user → agent → user → agent sequence", () => {
    const messages: MessageResponse[] = [
      {
        id: "user-1",
        chat_id: "chat-1",
        role: "user",
        content: "First question",
        created_at: "2026-01-01T00:00:00Z",
      },
      {
        id: "agent-1",
        chat_id: "chat-1",
        role: "agent",
        content: "First answer",
        agent_id: "system",
        status: "completed",
        created_at: "2026-01-01T00:00:01Z",
      },
      {
        id: "user-2",
        chat_id: "chat-1",
        role: "user",
        content: "Follow-up",
        created_at: "2026-01-01T00:00:02Z",
      },
      {
        id: "agent-2",
        chat_id: "chat-1",
        role: "agent",
        content: "Second answer",
        agent_id: "system",
        status: "completed",
        tool_calls: [
          makeToolCall({ id: "tc-1", provider_call_id: "tc-1", name: "cli", result: "output" }),
        ],
        created_at: "2026-01-01T00:00:03Z",
      },
    ];

    // --- SSE path ---
    const sseStore = new ChatStore();
    sseStore.handleEvent({ type: "chat_message", message: messages[0] });
    sseStore.handleEvent({ type: "token", content: "First answer" });
    sseStore.handleEvent({ type: "inference_done", message: messages[1] });
    sseStore.handleEvent({ type: "chat_message", message: messages[2] });
    sseStore.handleEvent({ type: "tool_call", id: "te-1", provider_call_id: "tc-1", name: "cli", arguments: "{}" });
    sseStore.handleEvent({ type: "tool_result", name: "cli", success: true, summary: "output" });
    sseStore.handleEvent({ type: "token", content: "Second answer" });
    sseStore.handleEvent({ type: "inference_done", message: messages[3] });

    const sseConverted = sseStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [...messages];
    loadedStore.loaded = true;

    const loadedConverted = loadedStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    expect(sseConverted).toEqual(loadedConverted);
  });
});

// ---------------------------------------------------------------------------
// Parity: consecutive agent messages (merge behavior)
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: consecutive agent messages merged", () => {
  it("merged messages produce the same converted output", () => {
    const messages: MessageResponse[] = [
      {
        id: "agent-1",
        chat_id: "chat-1",
        role: "agent",
        content: "Part one",
        agent_id: "system",
        status: "completed",
        created_at: "2026-01-01T00:00:00Z",
      },
      {
        id: "agent-2",
        chat_id: "chat-1",
        role: "agent",
        content: "Part two",
        agent_id: "system",
        status: "completed",
        created_at: "2026-01-01T00:00:01Z",
      },
    ];

    // --- SSE path ---
    const sseStore = new ChatStore();
    sseStore.handleEvent({ type: "token", content: "Part one" });
    sseStore.handleEvent({ type: "inference_done", message: messages[0] });
    sseStore.handleEvent({ type: "token", content: "Part two" });
    sseStore.handleEvent({ type: "inference_done", message: messages[1] });

    const sseDisplay = sseStore.getDisplayMessages();

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [...messages];
    loadedStore.loaded = true;

    const loadedDisplay = loadedStore.getDisplayMessages();

    // Both should mark the second message as a continuation
    expect(sseDisplay).toHaveLength(2);
    expect(loadedDisplay).toHaveLength(2);
    expect((sseDisplay[1] as any)._continuation).toBe(true);
    expect((loadedDisplay[1] as any)._continuation).toBe(true);

    const sseConverted = normalize(convertMessage(sseDisplay[0]));
    const loadedConverted = normalize(convertMessage(loadedDisplay[0]));
    expect(sseConverted).toEqual(loadedConverted);
  });
});

// ---------------------------------------------------------------------------
// Parity: reasoning + text + tools
// ---------------------------------------------------------------------------

describe("SSE-vs-Loaded parity: reasoning + text + tools", () => {
  it("produces identical output with all content types", () => {
    const agentMsg: MessageResponse = {
      id: "agent-1",
      chat_id: "chat-1",
      role: "agent",
      content: "Here's what I found.",
      agent_id: "system",
      reasoning: "Let me think step by step.",
      status: "completed",
      tool_calls: [
        makeToolCall({
          id: "tc-1",
          provider_call_id: "tc-1",
          name: "web_search",
          result: "Results",
        }),
      ],
      created_at: "2026-01-01T00:00:01Z",
    };

    // --- SSE path ---
    const sseStore = new ChatStore();
    sseStore.handleEvent({ type: "reasoning", content: "Let me think step by step." });
    sseStore.handleEvent({ type: "tool_call", id: "te-1", provider_call_id: "tc-1", name: "web_search", arguments: '{"query":"test"}' });
    sseStore.handleEvent({ type: "tool_result", name: "web_search", success: true, summary: "Results" });
    sseStore.handleEvent({ type: "token", content: "Here's what I found." });
    sseStore.handleEvent({ type: "inference_done", message: agentMsg });

    const sseConverted = sseStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    // --- Loaded path ---
    const loadedStore = new ChatStore();
    loadedStore.messages = [agentMsg];
    loadedStore.loaded = true;

    const loadedConverted = loadedStore.getDisplayMessages().map((m) => normalize(convertMessage(m)));

    expect(sseConverted).toEqual(loadedConverted);
  });
});
