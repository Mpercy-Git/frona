"use client";

import { createContext, useContext, createElement } from "react";

interface ChatContextValue {
  chatId: string;
  agentId: string;
}

export const ChatContext = createContext<ChatContextValue | null>(null);

export function ChatProvider({
  chatId,
  agentId,
  children,
}: {
  chatId: string;
  agentId: string;
  children: React.ReactNode;
}) {
  return createElement(
    ChatContext.Provider,
    { value: { chatId, agentId } },
    children,
  );
}

export function useChat(): ChatContextValue {
  const ctx = useContext(ChatContext);
  if (!ctx) throw new Error("useChat must be used within ChatProvider");
  return ctx;
}
