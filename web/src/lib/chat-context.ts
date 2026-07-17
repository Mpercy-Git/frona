"use client";

import { createContext, useContext, createElement } from "react";
import type { Attachment } from "./types";

interface ChatContextValue {
  chatId: string;
  agentId: string;
  /** Send a steering message while the agent is running (interrupt + redirect). */
  interrupt?: (text: string, attachments?: Attachment[]) => void;
}

export const ChatContext = createContext<ChatContextValue | null>(null);

export function ChatProvider({
  chatId,
  agentId,
  interrupt,
  children,
}: {
  chatId: string;
  agentId: string;
  interrupt?: (text: string, attachments?: Attachment[]) => void;
  children: React.ReactNode;
}) {
  return createElement(
    ChatContext.Provider,
    { value: { chatId, agentId, interrupt } },
    children,
  );
}

export function useChat(): ChatContextValue {
  const ctx = useContext(ChatContext);
  if (!ctx) throw new Error("useChat must be used within ChatProvider");
  return ctx;
}
