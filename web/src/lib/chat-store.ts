import type { ChatSSEEvent } from "./sse-event-bus";
import type { MessageResponse, MessageStatus, Attachment, ToolCall } from "./types";
import { api } from "./api-client";

// The "executing → completed" branch handles legacy rows where Executing
// was used as an implicit paused indicator (server now sets Paused).
function optimisticStatusAfterResolve(
  current: MessageStatus | undefined,
  allResolved: boolean,
): MessageStatus | undefined {
  if (!allResolved) return current;
  if (current === "paused") return "executing";
  if (current === "executing") return "completed";
  return current;
}

interface ToolCallPart {
  type: "tool-call";
  id: string;
  providerCallId: string;
  toolName: string;
  args: Record<string, unknown>;
  argsText: string;
  result?: string;
  isError?: boolean;
}

export interface RetryInfo {
  reason: string;
  retryAfterSecs: number;
  startedAt: number;
}

export interface StoreSnapshot {
  messages: MessageResponse[];
  isRunning: boolean;
  loaded: boolean;
  retryInfo: RetryInfo | null;
  pendingTools: ToolCall[];
  hasMore: boolean;
  loadingMore: boolean;
}

/**
 * Per-chat reactive store that owns all message + streaming state.
 * SSE events are handled as simple state mutations + subscriber notifications.
 * React reads state via useSyncExternalStore.
 */
export class ChatStore {
  messages: MessageResponse[] = [];
  /** Text accumulated during the current inference stream. */
  streamingText = "";
  streamingReasoning = "";
  streamingToolCalls = new Map<string, ToolCallPart>();
  streamingToolResults = new Map<string, { result: string; isError: boolean }>();
  /** External tools waiting for user resolution (keyed by tool call id). */
  pendingExternalTools = new Map<string, ToolCall>();
  isRunning = false;
  retryInfo: RetryInfo | null = null;
  loaded = false;
  hasMore = false;
  loadingMore = false;

  /** Tracks text position at the time of each tool call for turnText extraction. */
  private lastTextSnapshot = 0;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private listeners = new Set<() => void>();
  private _snapshot: StoreSnapshot | null = null;

  subscribe(callback: () => void): () => void {
    this.listeners.add(callback);
    return () => this.listeners.delete(callback);
  }

  getSnapshot(): StoreSnapshot {
    if (!this._snapshot) {
      this._snapshot = {
        messages: this.getDisplayMessages(),
        isRunning: this.isRunning,
        loaded: this.loaded,
        retryInfo: this.retryInfo,
        pendingTools: this.getPendingExternalTools(),
        hasMore: this.hasMore,
        loadingMore: this.loadingMore,
      };
    }
    return this._snapshot;
  }

  private notify() {
    this._snapshot = null;
    for (const fn of this.listeners) fn();
  }

  markLoaded() {
    this.loaded = true;
    this.notify();
  }

  markRunning() {
    this.isRunning = true;
    this.notify();
  }

  getPendingExternalTools(): ToolCall[] {
    return [...this.pendingExternalTools.values()].filter(
      (te) => te.hitl?.status === "pending",
    );
  }

  /** Add a user message optimistically (before backend echo). */
  addUserMessage(content: string, attachments?: Attachment[]) {
    this.messages.push({
      id: `__user_${Date.now()}`,
      chat_id: "",
      role: "user",
      content,
      attachments,
      created_at: new Date().toISOString(),
    });
    this.isRunning = true;
    this.notify();
  }

  async loadMessages(chatId: string) {
    try {
      const { messages, has_more } = await api.get<{ messages: MessageResponse[]; has_more: boolean }>(
        `/api/chats/${chatId}/messages`,
      );
      // Merge instead of overwrite so messages that arrived first — via
      // buffered SSE events on an unloaded chat, or the optimistic user
      // message from onNew — survive the historical fetch. Historical
      // messages predate anything in the store, so they go in front.
      if (this.messages.length === 0) {
        this.messages = messages;
      } else {
        const existing = new Set(this.messages.map((m) => m.id));
        const historical = messages.filter((m) => !existing.has(m.id));
        this.messages = [...historical, ...this.messages];
      }
      this.hasMore = has_more;
    } catch {
      // leave any optimistic/SSE-delivered messages alone
    }
    this.hydrateExternalTools();
    this.loaded = true;
    this.notify();
  }

  async loadOlder(chatId: string) {
    if (this.loadingMore || !this.hasMore) return;
    const earliest = this.messages.find((m) => !m.id.startsWith("__"));
    if (!earliest) return;
    this.loadingMore = true;
    this.notify();
    try {
      const params = new URLSearchParams({ before: earliest.created_at, limit: "50" });
      const { messages, has_more } = await api.get<{ messages: MessageResponse[]; has_more: boolean }>(
        `/api/chats/${chatId}/messages?${params}`,
      );
      const existing = new Set(this.messages.map((m) => m.id));
      const older = messages.filter((m) => !existing.has(m.id));
      this.messages = [...older, ...this.messages];
      this.hasMore = has_more;
    } catch {
      // Keep hasMore as-is so a future scroll can retry.
    } finally {
      this.loadingMore = false;
      this.notify();
    }
  }

  /** Scan loaded messages for pending external tool calls and populate the map. */
  private hydrateExternalTools() {
    for (const msg of this.messages) {
      if (!msg.tool_calls) continue;
      for (const te of msg.tool_calls) {
        if (te.hitl && te.hitl.status === "pending") {
          this.pendingExternalTools.set(te.id, te);
        }
      }
    }
  }

  handleEvent(event: ChatSSEEvent) {
    switch (event.type) {
      case "token":
        this.isRunning = true;
        this.streamingText += event.content;
        break;

      case "reasoning":
        this.isRunning = true;
        this.streamingReasoning += event.content;
        break;

      case "tool_call": {
        this.isRunning = true;
        const args = tryParseJson(event.arguments);
        if (event.description) args.description = event.description;

        // Capture text spoken before this tool call as turnText
        if (this.streamingText.length > this.lastTextSnapshot) {
          const turnText = this.streamingText.slice(this.lastTextSnapshot).trim();
          if (turnText) args.turnText = turnText;
          this.lastTextSnapshot = this.streamingText.length;
        }

        this.streamingToolCalls.set(event.id, {
          type: "tool-call",
          id: event.id,
          providerCallId: event.provider_call_id,
          toolName: event.name,
          args,
          argsText: typeof event.arguments === "string"
            ? event.arguments
            : JSON.stringify(event.arguments ?? {}),
        });
        break;
      }

      case "tool_result": {
        // Match by tool name to the first unresolved tool call
        for (const [id, tc] of this.streamingToolCalls) {
          if (tc.toolName === event.name && !this.streamingToolResults.has(id)) {
            this.streamingToolResults.set(id, {
              result: event.summary ?? (event.success ? "Done" : "Error"),
              isError: !event.success,
            });
            break;
          }
        }
        break;
      }

      case "inference_start": {
        // Channel adapters use this to start typing/thinking affordance.
        // FE already sets isRunning=true on optimistic user message; this
        // is just a no-op confirmation. Streaming events that follow keep
        // isRunning=true via their existing handlers.
        this.isRunning = true;
        break;
      }

      case "inference_paused": {
        // Loop parked waiting for something external. Universal lifecycle:
        // replace message, drop spinner. Reason-specific UI is dispatched
        // off `event.reason.type` — adding a new pause cause is a new
        // branch here.
        const msg = event.message;
        const existingIdx = this.messages.findIndex((m) => m.id === msg.id);
        if (existingIdx >= 0) {
          this.messages[existingIdx] = msg;
        } else {
          this.messages.push(msg);
        }
        this.clearStreaming();

        switch (event.reason.type) {
          case "Hitl":
            // Hydrate pending HITLs from msg.tool_calls so the wizard renders.
            if (msg.tool_calls?.length) {
              for (const te of msg.tool_calls) {
                if (te.hitl && te.hitl.status === "pending") {
                  this.pendingExternalTools.set(te.id, te);
                }
              }
            }
            break;
        }
        break;
      }

      case "inference_resume": {
        // Human just resolved a HITL — message reflects the post-resolution
        // state. Replace message and update pendingExternalTools for the
        // resolved tool_call so its `hitl.status` flips to resolved/denied
        // in the wizard view.
        const msg = event.message;
        const idx = this.messages.findIndex((m) => m.id === msg.id);
        if (idx >= 0) {
          // Preserve tool_calls — backend MessageResponse always has them empty for this path.
          if ((!msg.tool_calls || msg.tool_calls.length === 0) && this.messages[idx].tool_calls?.length) {
            msg.tool_calls = this.messages[idx].tool_calls;
          }
          this.messages[idx] = msg;
        }
        if (msg.tool_calls?.length) {
          for (const te of msg.tool_calls) {
            if (this.pendingExternalTools.has(te.id)) {
              this.pendingExternalTools.set(te.id, te);
            }
          }
        }
        break;
      }

      case "inference_done": {
        const msg = event.message;

        // If backend sent tool_calls, use them as-is (source of truth).
        // Otherwise fall back to streaming state (interactive chats where complete_agent_message doesn't populate them).
        if (!msg.tool_calls?.length) {
          const streamingTools = this.buildToolCalls();
          if (streamingTools.length > 0) {
            msg.tool_calls = streamingTools;
          }
        }

        const existingIdx = this.messages.findIndex((m) => m.id === msg.id);
        if (existingIdx >= 0) {
          this.messages[existingIdx] = msg;
        } else {
          this.messages.push(msg);
        }
        this.clearStreaming();

        // The agent loop can pause on a fresh set of HITLs and signal that via
        // inference_done (vs. inference_paused). Re-seed the pending map so the
        // wizard renders the new questions; matches the inference_paused path.
        if (msg.tool_calls?.length) {
          for (const te of msg.tool_calls) {
            if (te.hitl && te.hitl.status === "pending") {
              this.pendingExternalTools.set(te.id, te);
            }
          }
        }
        break;
      }

      case "chat_message": {
        // Replace optimistic user message with the real one from the backend
        if (event.message.role === "user") {
          const optIdx = this.messages.findIndex((m) => m.id.startsWith("__user_"));
          if (optIdx >= 0) {
            this.messages[optIdx] = event.message;
            break;
          }
        }
        // Skip if this message ID is already in the array
        if (!this.messages.some((m) => m.id === event.message.id)) {
          this.messages.push(event.message);
        }
        break;
      }

      case "retry": {
        this.retryInfo = {
          reason: event.reason,
          retryAfterSecs: event.retryAfterSecs,
          startedAt: Date.now(),
        };
        // Auto-clear after the retry period
        if (this.retryTimer) clearTimeout(this.retryTimer);
        const capturedStart = this.retryInfo.startedAt;
        this.retryTimer = setTimeout(() => {
          if (this.retryInfo?.startedAt === capturedStart) {
            this.retryInfo = null;
            this.notify();
          }
        }, event.retryAfterSecs * 1000 + 500);
        break;
      }

      case "inference_cancelled":
      case "inference_error":
        this.clearStreaming();
        break;
    }
    this.notify();
  }

  /**
   * Build the display message list. During streaming, appends a synthetic
   * "in-progress" assistant message built from the streaming state.
   */
  getDisplayMessages(): MessageResponse[] {
    const merged = mergeConsecutiveMessages(this.messages);
    if (!this.isRunning) return merged;

    const displayText = this.lastTextSnapshot > 0
      ? this.streamingText.slice(this.lastTextSnapshot)
      : this.streamingText;

    const streamingTools = this.buildToolCalls();

    // If the last message is already executing (e.g. page refreshed mid-inference),
    // merge new streaming state into it instead of appending a separate message.
    const last = merged[merged.length - 1];
    if (last?.status === "executing" && last.role === "agent") {
      const updated = { ...last };
      if (displayText) {
        updated.content = [last.content, displayText].filter(Boolean).join("");
      }
      if (this.streamingReasoning) {
        updated.reasoning = [last.reasoning, this.streamingReasoning].filter(Boolean).join("");
      }
      if (streamingTools.length > 0) {
        updated.tool_calls = [...(last.tool_calls ?? []), ...streamingTools];
      }
      return [...merged.slice(0, -1), updated];
    }

    const syntheticMessage: MessageResponse = {
      id: "__streaming__",
      chat_id: "",
      role: "agent",
      content: displayText,
      reasoning: this.streamingReasoning || undefined,
      tool_calls: streamingTools.length > 0 ? streamingTools : undefined,
      status: "executing",
      created_at: new Date().toISOString(),
    };

    return [...merged, syntheticMessage];
  }

  private buildToolCalls(): ToolCall[] {
    const result: ToolCall[] = [];
    for (const tc of this.streamingToolCalls.values()) {
      const pending = this.pendingExternalTools.get(tc.id);
      if (pending) {
        result.push(pending);
      } else {
        const res = this.streamingToolResults.get(tc.id);
        result.push({
          id: tc.id,
          chat_id: "",
          message_id: "",
          turn: 0,
          provider_call_id: tc.providerCallId,
          name: tc.toolName,
          arguments: tc.args as Record<string, unknown>,
          result: res?.result ?? "",
          success: res ? !res.isError : true,
          duration_ms: 0,
          description: tc.args.description as string | undefined,
          turn_text: tc.args.turnText as string | undefined,
          created_at: "",
        });
      }
    }
    return result;
  }

  resolveToolCall(toolCallId: string, result: string) {
    // Check pending external tools (streaming state) — optimistic update
    const pending = this.pendingExternalTools.get(toolCallId);
    if (pending?.hitl) {
      this.pendingExternalTools.set(toolCallId, {
        ...pending,
        result,
        hitl: { ...pending.hitl, status: "resolved" },
      });
      this.notify();
      return;
    }

    // Fall through to message-based resolution
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const msg = this.messages[i];
      if (!msg.tool_calls) continue;
      const teIdx = msg.tool_calls.findIndex(
        (t) => t.id === toolCallId || t.provider_call_id === toolCallId,
      );
      if (teIdx < 0) continue;
      const te = msg.tool_calls[teIdx];
      const updatedTe = te.hitl
        ? { ...te, result, hitl: { ...te.hitl, status: "resolved" as const } }
        : { ...te, result };
      const updatedCalls = [...msg.tool_calls];
      updatedCalls[teIdx] = updatedTe;
      const allResolved = updatedCalls.every(
        (t) => !t.hitl || t.hitl.status !== "pending",
      );
      this.messages[i] = {
        ...msg,
        tool_calls: updatedCalls,
        status: optimisticStatusAfterResolve(msg.status, allResolved),
      };
      this.notify();
      return;
    }
  }

  private updateToolCall(te: ToolCall) {
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const msg = this.messages[i];
      if (!msg.tool_calls) continue;
      const teIdx = msg.tool_calls.findIndex((t) => t.id === te.id);
      if (teIdx >= 0) {
        const updatedCalls = [...msg.tool_calls];
        updatedCalls[teIdx] = te;
        const allResolved = updatedCalls.every(
          (t) => !t.hitl || t.hitl.status !== "pending",
        );
        this.messages[i] = {
          ...msg,
          tool_calls: updatedCalls,
          status: optimisticStatusAfterResolve(msg.status, allResolved),
        };
        break;
      }
    }
  }

  clearStreaming() {
    this.isRunning = false;
    this.streamingText = "";
    this.streamingReasoning = "";
    this.streamingToolCalls.clear();
    this.streamingToolResults.clear();
    this.pendingExternalTools.clear();
    this.lastTextSnapshot = 0;
    this.retryInfo = null;
    if (this.retryTimer) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
  }
}

/**
 * Merge consecutive agent messages from the same agent into a single message.
 * This prevents fragmentation when the backend sends multiple messages in sequence.
 */
/**
 * Tag consecutive agent messages from the same agent so the UI can hide
 * repeated headers. No content merging — each message keeps its own order.
 */
export function mergeConsecutiveMessages(messages: MessageResponse[]): MessageResponse[] {
  const result: MessageResponse[] = [];
  for (const msg of messages) {
    const prev = result[result.length - 1];
    const isContinuation =
      prev &&
      prev.role === "agent" &&
      (msg.role === "agent" || (msg.role === "system" && msg.event)) &&
      (prev.agent_id === msg.agent_id || msg.role === "system") &&
      prev.status !== "executing";

    if (isContinuation) {
      result.push({ ...msg, _continuation: true } as MessageResponse);
    } else {
      result.push(msg);
    }
  }
  return result;
}

function tryParseJson(value: unknown): Record<string, unknown> {
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value);
      if (typeof parsed === "object" && parsed !== null) return parsed;
      return {};
    } catch {
      return {};
    }
  }
  if (typeof value === "object" && value !== null) return value as Record<string, unknown>;
  return {};
}
