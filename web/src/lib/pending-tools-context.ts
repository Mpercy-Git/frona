"use client";

import { createContext, useContext } from "react";
import type { ToolCall } from "./types";

export const PendingToolsContext = createContext<ToolCall[]>([]);

export function usePendingTools(): ToolCall[] {
  return useContext(PendingToolsContext);
}
