"use client";

import { useMemo, useEffect, useCallback, useRef, useSyncExternalStore } from "react";
import { useExternalStoreRuntime } from "@assistant-ui/react";
import type { CompleteAttachment, AppendMessage, AttachmentAdapter, PendingAttachment } from "@assistant-ui/react";
import type { ExternalStoreAdapter } from "@assistant-ui/react";
import { ChatStore, type RetryInfo } from "./chat-store";
import { sseBus } from "./sse-event-bus";
import { sendMessage as apiSendMessage, cancelGeneration, api, uploadFile } from "./api-client";
import { computeTimeMarkers, useTimezone } from "./format-time";
import type { MessageResponse, ChatResponse, Attachment } from "./types";
import { renderMessageBody } from "./task-result-render";
import { useToast } from "./toast";


const backendAttachmentRegistry = new Map<string, Attachment>();

export function registerBackendAttachment(id: string, attachment: Attachment) {
  backendAttachmentRegistry.set(id, attachment);
}

export function getBackendAttachment(id: string): Attachment | undefined {
  return backendAttachmentRegistry.get(id);
}

function convertBackendAttachment(att: Attachment): CompleteAttachment {
  const url = att.url ?? "";
  // Only treat it as a renderable image if we actually have a URL — otherwise
  // an empty src produces a broken <img>. Fall back to the file placeholder.
  const isImage = att.content_type.startsWith("image/") && url !== "";
  registerBackendAttachment(att.path, att);
  return {
    id: att.path,
    type: isImage ? "image" : "file",
    name: att.filename,
    contentType: att.content_type,
    status: { type: "complete" },
    content: isImage
      ? [{ type: "image", image: url }]
      : [{ type: "text", text: `[file: ${att.filename}]` }],
  };
}

export function createFronaAttachmentAdapter(
  onError?: (message: string) => void,
): AttachmentAdapter {
  return {
  // assistant-ui's fileMatchesAccept treats ONLY the exact string "*" as
  // "accept anything". "*/*" fails the match for every real MIME type (e.g.
  // image/jpeg), so the composer silently rejects the file with a swallowed
  // "not-accepted" error — the user selects a file and nothing appears. This
  // was most visible on mobile, where attaching a photo did nothing.
  accept: "*",

  async add({ file }: { file: File }): Promise<PendingAttachment> {
    let uploaded: Attachment;
    try {
      uploaded = await uploadFile(file);
    } catch (e) {
      // The runtime swallows adapter errors (only emitting an event), so
      // without this the upload just vanishes — e.g. a phone photo over the
      // 10MB cap. Surface it, then re-throw so the attachment isn't kept.
      const detail = e instanceof Error ? e.message : "Upload failed";
      onError?.(`Couldn't upload ${file.name}: ${detail}`);
      throw e;
    }
    backendAttachmentRegistry.set(uploaded.path, uploaded);
    return {
      id: uploaded.path,
      type: "file",
      name: uploaded.filename,
      contentType: uploaded.content_type,
      status: { type: "requires-action", reason: "composer-send" },
      content: [],
      file,
    };
  },

  async send(attachment: PendingAttachment): Promise<CompleteAttachment> {
    const isImage = attachment.contentType?.startsWith("image/");
    // Prefer the presigned URL from the upload response over a blob URL: it
    // needs no revoking (no leak) and matches what history reload will show.
    const backend = backendAttachmentRegistry.get(attachment.id);
    const imageUrl = backend?.url || (attachment.file ? URL.createObjectURL(attachment.file) : undefined);
    const renderAsImage = isImage && imageUrl != null;
    return {
      ...attachment,
      // Upgrade to "image" for renderable images so the completed attachment
      // matches what convertBackendAttachment produces on history reload.
      type: renderAsImage ? "image" : "file",
      status: { type: "complete" },
      content: renderAsImage
        ? [{ type: "image", image: imageUrl }]
        : [{ type: "text", text: `[file: ${attachment.name}]` }],
    };
  },

  async remove(attachment) {
    backendAttachmentRegistry.delete(attachment.id);
  },
  };
}

/** Default adapter with no error surfacing (kept for non-hook callers). */
export const fronaAttachmentAdapter: AttachmentAdapter = createFronaAttachmentAdapter();


export type AssistantContentPart =
  | { type: "text"; text: string }
  | { type: "reasoning"; text: string }
  | { type: "tool-call"; toolCallId: string; toolName: string; args: Record<string, string | number | boolean | null>; argsText: string; result?: string };

/**
 * If the text part is empty but a tool call has turnText, promote the last
 * turnText to the main text and strip it from all tool call args.
 */
export function promoteTurnText(parts: AssistantContentPart[]): AssistantContentPart[] {
  const textPart = parts.find((p) => p.type === "text");
  if (textPart && "text" in textPart && textPart.text.trim()) return parts;

  let lastTurnText = "";
  for (const p of parts) {
    if (p.type === "tool-call" && typeof (p.args as Record<string, unknown>)?.turnText === "string") {
      lastTurnText = (p.args as Record<string, unknown>).turnText as string;
    }
  }
  if (!lastTurnText) return parts;

  return parts.map(p => {
    if (p.type === "text") return { ...p, text: lastTurnText };
    if (p.type === "tool-call" && (p.args as Record<string, unknown>)?.turnText) {
      const { turnText: _, ...rest } = p.args;
      return { ...p, args: rest };
    }
    return p;
  });
}

export function convertMessage(msg: MessageResponse) {
  if (msg.role === "user" || msg.role === "contact" || msg.role === "livecall") {
    const attachments = msg.attachments?.map(convertBackendAttachment);
    return {
      id: msg.id,
      role: "user" as const,
      content: [{ type: "text" as const, text: msg.content || "" }],
      createdAt: new Date(msg.created_at),
      ...(attachments?.length ? { attachments } : {}),
      metadata: {
        custom: {
          originalRole: msg.role,
          contactId: msg.contact_id,
          daySeparator: msg._daySeparator,
          gap: msg._gap,
          command: msg.command,
        },
      },
    };
  }

  if (msg.role === "agent" || msg.role === "taskcompletion" || (msg.role === "system" && msg.event)) {
    // TaskCompletion with nothing to show (complex schema with no recognized
    // text field, no attachments) → suppress the bubble entirely. Failed
    // tasks still surface so the user can see the failure.
    if (
      msg.role === "taskcompletion" &&
      msg.event?.type === "TaskCompletion" &&
      msg.event.data.status !== "Failed" &&
      !msg.attachments?.length &&
      !renderMessageBody(msg).trim()
    ) {
      return null;
    }

    const content: AssistantContentPart[] = [];

    // User-facing external tools (HITL prompts) render BEFORE text.
    if (msg.tool_calls?.length) {
      for (const te of msg.tool_calls) {
        if (te.hitl) {
          const toolName = te.hitl.request.type;
          const status = te.hitl.status;
          const resolved = status === "resolved" || status === "denied";
          // Project request data + hitl-level fields into a flat args object
          // for assistant-ui consumption.
          const args: Record<string, string | number | boolean | null> = {
            prompt: te.hitl.prompt,
            url: te.hitl.url,
            status,
          };
          for (const [k, v] of Object.entries(te.hitl.request.data)) {
            if (v == null || typeof v === "string" || typeof v === "number" || typeof v === "boolean") {
              args[k] = v as string | number | boolean | null;
            } else {
              args[k] = JSON.stringify(v);
            }
          }
          const responseText =
            te.hitl.response == null
              ? null
              : te.hitl.response.type === "Approval"
                ? te.hitl.response.data
                  ? "Approved"
                  : "Denied"
                : te.hitl.response.type === "Choice"
                  ? te.hitl.response.data
                  : te.hitl.response.data.type === "Granted"
                    ? "Granted"
                    : te.hitl.response.data.type === "GrantedMany"
                      ? `Granted ${te.hitl.response.data.data.grants.length}`
                      : "Denied";
          content.push({
            type: "tool-call",
            toolCallId: te.id,
            toolName,
            args,
            argsText: JSON.stringify(args),
            ...(resolved && responseText != null ? { result: responseText } : {}),
            ...(resolved && responseText == null ? { result: String(status) } : {}),
          });
        }
      }
    }

    const bodyText = renderMessageBody(msg);
    if (bodyText) {
      content.push({ type: "text", text: bodyText });
    }
    if (!bodyText && !msg.reasoning && !msg.event) {
      content.push({ type: "text", text: "" });
    }

    // Attachments render between text and tools
    if (msg.attachments?.length) {
      content.push({
        type: "tool-call",
        toolCallId: `__attachments_${msg.id}`,
        toolName: "Attachments",
        args: {} as Record<string, string | number | boolean | null>,
        argsText: "{}",
        result: "done",
      });
    }

    // Task lifecycle events (TaskCompletion / TaskDeferred) — after
    // attachments, before regular tools.
    if (msg.tool_calls?.length) {
      for (const te of msg.tool_calls) {
        if (te.task_event) {
          const toolName = te.task_event.type;
          const data = te.task_event.data as Record<string, unknown>;
          const args: Record<string, string | number | boolean | null> = {};
          for (const [k, v] of Object.entries(data)) {
            if (v == null || typeof v === "string" || typeof v === "number" || typeof v === "boolean") {
              args[k] = v as string | number | boolean | null;
            } else {
              args[k] = JSON.stringify(v);
            }
          }
          content.push({
            type: "tool-call",
            toolCallId: te.id,
            toolName,
            args,
            argsText: JSON.stringify(args),
            result: String(data.status ?? "done"),
          });
        }
      }
    }

    // Regular tools (neither hitl nor task_event) — last
    if (msg.tool_calls?.length) {
      for (const te of msg.tool_calls) {
        if (!te.hitl && !te.task_event) {
          content.push({
            type: "tool-call",
            toolCallId: te.id,
            toolName: te.name,
            args: {
              description: te.description ?? te.name,
              ...te.arguments,
              ...(te.turn_text ? { turnText: te.turn_text } : {}),
              ...(!te.success ? { isError: true } : {}),
            } as Record<string, string | number | boolean | null>,
            argsText: JSON.stringify(te.arguments || {}),
            result: te.result,
          });
        }
      }
    }

    // Mid-flight, keep turnText in tool-call args so they render as bubbles
    // between tools; only promote it when the message is fully done.
    const inFlight = msg.status === "executing" || msg.status === "paused";
    const finalContent = inFlight ? content : promoteTurnText(content);

    return {
      id: msg.id,
      role: "assistant" as const,
      content: finalContent,
      createdAt: new Date(msg.created_at),
      status: msg.tool_calls?.some(te => te.hitl?.status === "pending")
        ? { type: "requires-action" as const, reason: "tool-calls" as const }
        : inFlight
          ? { type: "running" as const }
          : { type: "complete" as const, reason: "stop" as const },
      metadata: {
        custom: {
          agentId: msg.agent_id,
          originalRole: msg.role,
          continuation: msg._continuation,
          daySeparator: msg._daySeparator,
          gap: msg._gap,
          ...(msg.reasoning ? { reasoning: msg.reasoning } : {}),
          ...(msg.attachments?.length ? { attachments: msg.attachments } : {}),
          ...(msg.role === "taskcompletion" && msg.event?.type === "TaskCompletion"
            ? {
                taskCompletion: {
                  task_id: msg.event.data.task_id,
                  status: msg.event.data.status,
                },
              }
            : {}),
        },
      },
    };
  }

  // System messages without events — skip
  return null;
}


export interface ChatRuntimeOptions {
  chatId?: string;
  agentId: string;
  onChatCreated?: (chat: ChatResponse) => void;
}

export function useChatRuntime({ chatId, agentId, onChatCreated }: ChatRuntimeOptions) {
  const toast = useToast();
  const currentChatIdRef = useRef<string | null>(chatId ?? null);
  currentChatIdRef.current = chatId ?? currentChatIdRef.current;
  const onChatCreatedRef = useRef(onChatCreated);
  onChatCreatedRef.current = onChatCreated;

  // One store per ChatView mount — persists across chatId changes (pending → real)
  const storeRef = useRef<ChatStore | null>(null);
  if (!storeRef.current) {
    storeRef.current = new ChatStore();
  }
  const store = storeRef.current;

  // Eager SSE subscription controller — set in onNew so events are captured
  // immediately, before the useEffect has a chance to fire.
  const eagerSubRef = useRef<AbortController | null>(null);

  // Subscribe to store changes for re-rendering
  const subscribe = useCallback((cb: () => void) => store.subscribe(cb), [store]);
  const storeSnapshot = useSyncExternalStore(
    subscribe,
    () => store.getSnapshot(),
  );

  // Load messages for existing chats. Skip if the store already has messages
  // (new chat: optimistic user message was added before chatId existed).
  useEffect(() => {
    if (chatId && store.messages.length === 0) {
      store.loadMessages(chatId);
    } else {
      store.markLoaded();
    }
  }, [chatId, store]);

  // Subscribe to SSE events for the chat.
  // If an eager subscription was started in onNew, adopt it instead of creating a new one.
  useEffect(() => {
    if (!chatId) return;

    if (eagerSubRef.current) {
      const adopted = eagerSubRef.current;
      eagerSubRef.current = null;
      return () => adopted.abort();
    }

    const controller = new AbortController();
    const events = sseBus.subscribe(chatId, controller.signal);

    (async () => {
      for await (const event of events) {
        store.handleEvent(event);
      }
    })();

    return () => controller.abort();
  }, [store, chatId]);

  useEffect(() => {
    return sseBus.onReconnect(() => {
      const id = currentChatIdRef.current;
      if (id) store.loadMessages(id);
    });
  }, [store]);

  // onNew callback — creates chat if needed, sends message to backend
  const onNew = useCallback(async (message: AppendMessage) => {
    const text = message.content
      .filter((p): p is { type: "text"; text: string } => p.type === "text")
      .map((p) => p.text)
      .join("");

    const attachments: Attachment[] = [];
    const missingAttachments: string[] = [];
    if ("attachments" in message && message.attachments) {
      for (const att of message.attachments) {
        const backend = backendAttachmentRegistry.get(att.id);
        if (backend) {
          attachments.push(backend);
        } else {
          // Registry miss (e.g. page reload while a file was staged). Do NOT
          // throw here: onNew is the runtime's append callback, and rejecting
          // it makes assistant-ui roll back the optimistic message, which
          // corrupts its internal tap client list ("Index out of bounds").
          // Record the miss and surface it non-fatally instead of crashing.
          missingAttachments.push(att.name ?? att.id);
        }
      }
      if (missingAttachments.length > 0) {
        toast.error(
          `Couldn't attach ${missingAttachments.join(", ")} — please re-attach and resend.`,
        );
      }
    }

    let sendChatId = currentChatIdRef.current;
    if (!sendChatId) {
      // Standalone composers (home/space page) handle chat creation themselves.
      // Only create a chat here if there's a promotion callback.
      if (!onChatCreatedRef.current) return;
      const chat = await api.post<ChatResponse>("/api/chats", { agent_id: agentId });
      sendChatId = chat.id;
      currentChatIdRef.current = sendChatId;

      // Eagerly subscribe to SSE events NOW, before the React effect fires.
      // This eliminates the race window where events could arrive unbuffered.
      const eager = new AbortController();
      eagerSubRef.current = eager;
      const events = sseBus.subscribe(sendChatId, eager.signal);
      (async () => {
        for await (const event of events) {
          store.handleEvent(event);
        }
      })();

      // This triggers slot promotion → chatId prop change → SSE effect adopts the eager sub.
      onChatCreatedRef.current(chat);
    }

    store.addUserMessage(text, attachments.length ? attachments : undefined);

    const body = attachments.length
      ? { content: text, attachments }
      : { content: text };

    try {
      await apiSendMessage(sendChatId, body);
    } catch {
      store.clearStreaming();
    }
  }, [agentId, store, toast]);

  const onCancel = useCallback(async () => {
    const id = currentChatIdRef.current;
    if (id) {
      await cancelGeneration(id).catch(() => {});
    }
  }, []);

  // Send a steering message *while the agent is running*. The backend registers
  // a fresh cancellation token per turn, which cancels the in-flight turn and
  // starts a new one that includes this message — so the agent keeps its full
  // history but gets redirected. Used to nudge a run that's gone off on a
  // tangent without losing the conversation.
  const interrupt = useCallback(
    (text: string, attachments: Attachment[] = []) => {
      const id = currentChatIdRef.current;
      const trimmed = text.trim();
      if (!id || !trimmed) return;
      store.addUserMessage(trimmed, attachments.length ? attachments : undefined);
      const body = attachments.length
        ? { content: trimmed, attachments }
        : { content: trimmed };
      apiSendMessage(id, body).catch(() => {
        toast.error("Couldn't send your message — please try again.");
      });
    },
    [store, toast],
  );

  const timeZone = useTimezone();

  // Filter out messages that convertMessage returns null for (e.g. signal-only
  // task completions) and annotate each with day-boundary / large-gap markers.
  // Markers compute on the post-filter list so a hidden message can't create
  // a phantom separator.
  const filteredMessages = useMemo(() => {
    const kept = storeSnapshot.messages.filter((msg) => convertMessage(msg) !== null);
    const markers = computeTimeMarkers(kept, timeZone);
    // Annotate each assistant message with the prior message's command (if any),
    // so the bubble renders as a compact command-response when applicable.
    return kept.map((msg) => {
      const marker = markers.get(msg.id);
      if (!marker) return msg;
      return { ...msg, _daySeparator: marker.daySeparator, _gap: marker.gap };
    });
  }, [storeSnapshot.messages, timeZone]);

  const attachmentAdapter = useMemo(
    () => createFronaAttachmentAdapter((message) => toast.error(message)),
    [toast],
  );

  const adapter: ExternalStoreAdapter<MessageResponse> = useMemo(() => ({
    messages: filteredMessages,
    isRunning: storeSnapshot.isRunning,
    convertMessage: (msg: MessageResponse) => convertMessage(msg) ?? {
      id: msg.id,
      role: "assistant" as const,
      content: [],
      createdAt: new Date(msg.created_at),
      status: { type: "complete" as const, reason: "stop" as const },
    },
    onNew,
    onCancel,
    onAddToolResult: ({ toolCallId, result }) => {
      store.resolveToolCall(toolCallId, String(result ?? ""));
    },
    adapters: {
      attachments: attachmentAdapter,
    },
  }), [filteredMessages, storeSnapshot.isRunning, onNew, onCancel, store, attachmentAdapter]);

  const runtime = useExternalStoreRuntime(adapter);

  // Programmatic send — used for pending messages
  const sendMessage = useCallback((content: string, attachments?: Attachment[]) => {
    if (attachments?.length) {
      for (const att of attachments) {
        registerBackendAttachment(att.path, att);
      }
    }
    runtime.thread.append({
      role: "user",
      content: [{ type: "text", text: content }],
      attachments: attachments?.map(convertBackendAttachment),
    });
  }, [runtime]);

  const loadOlder = useCallback(() => {
    const id = currentChatIdRef.current;
    if (id) store.loadOlder(id);
  }, [store]);

  return {
    runtime,
    loaded: storeSnapshot.loaded,
    sendMessage,
    interrupt,
    retryInfo: storeSnapshot.retryInfo,
    pendingTools: storeSnapshot.pendingTools,
    hasMore: storeSnapshot.hasMore,
    loadingMore: storeSnapshot.loadingMore,
    loadOlder,
    usagePerChat: storeSnapshot.usagePerChat,
    lastFallbackIndex: storeSnapshot.lastFallbackIndex,
    lastChatInputTokens: storeSnapshot.lastChatInputTokens,
    totalToolCalls: storeSnapshot.totalToolCalls,
  };
}

export type { RetryInfo };
