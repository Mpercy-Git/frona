"use client";

import { createContext, useContext } from "react";
import type { RetryInfo } from "./chat-store";

export const RetryContext = createContext<RetryInfo | null>(null);

export function useRetryInfo(): RetryInfo | null {
  return useContext(RetryContext);
}
