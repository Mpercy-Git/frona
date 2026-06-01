"use client";

import { createContext, useContext } from "react";

export interface ChatPagination {
  hasMore: boolean;
  loadingMore: boolean;
  loadOlder: () => void;
}

export const ChatPaginationContext = createContext<ChatPagination>({
  hasMore: false,
  loadingMore: false,
  loadOlder: () => {},
});

export function useChatPagination(): ChatPagination {
  return useContext(ChatPaginationContext);
}
