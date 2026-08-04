"use client";

import { createContext, useContext, createElement } from "react";
import type { Attachment } from "./types";

interface ChatContextValue {
  chatId: string;
  agentId: string;
  /** Send a steering message while the agent is running (interrupt + redirect). */
  interrupt?: (text: string, attachments?: Attachment[]) => void;
  /** True when this chat was shared with the viewer (read-only) rather than
   *  owned by them — composer and HITL resolution are hidden. The backend
   *  independently rejects mutations from non-owners; this only drives UI. */
  isReadOnly?: boolean;
}

export const ChatContext = createContext<ChatContextValue | null>(null);

export function ChatProvider({
  chatId,
  agentId,
  interrupt,
  isReadOnly,
  children,
}: {
  chatId: string;
  agentId: string;
  interrupt?: (text: string, attachments?: Attachment[]) => void;
  isReadOnly?: boolean;
  children: React.ReactNode;
}) {
  return createElement(
    ChatContext.Provider,
    { value: { chatId, agentId, interrupt, isReadOnly } },
    children,
  );
}

export function useChat(): ChatContextValue {
  const ctx = useContext(ChatContext);
  if (!ctx) throw new Error("useChat must be used within ChatProvider");
  return ctx;
}
