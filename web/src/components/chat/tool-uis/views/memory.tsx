"use client";

import type { FC } from "react";
import { ToolRow } from "./tool-row";
import type { ToolView, ToolViewProps } from "./types";

function firstLine(text: string, maxLen: number): string {
  const line = text.split("\n").find((l) => l.trim().length > 0)?.trim() ?? "";
  if (line.length <= maxLen) return line;
  return line.slice(0, maxLen - 1) + "…";
}

function makeMemoryView(title: string): ToolView {
  const Component: FC<ToolViewProps> = ({
    args,
    status,
    isExpanded,
    onToggle,
  }) => {
    const a = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
    const memory = typeof a.memory === "string" ? a.memory.trim() : "";
    const subtitle = memory ? firstLine(memory, 80) : "";
    // Only expandable when the body would show more than the subtitle already does.
    const expandable = memory.length > 0 && memory !== subtitle;

    return (
      <ToolRow status={status} expandable={expandable}>
        <ToolRow.Header onToggle={onToggle} isExpanded={isExpanded}>
          <ToolRow.Title>{title}</ToolRow.Title>
          <ToolRow.Subtitle>{subtitle || null}</ToolRow.Subtitle>
        </ToolRow.Header>
        <ToolRow.Body isExpanded={isExpanded} unstyled>
          <div className="p-3 text-xs">
            <p className="whitespace-pre-wrap text-text-secondary m-0">{memory}</p>
          </div>
        </ToolRow.Body>
      </ToolRow>
    );
  };
  Component.displayName = `MemoryView(${title})`;
  return Component;
}

export const StoreAgentMemoryView = makeMemoryView("Remember");
export const StoreUserMemoryView = makeMemoryView("Remember about user");

/**
 * Auto-expand memory rows whose content doesn't fit in a single-line subtitle —
 * multi-line entries or anything longer than ~100 chars. Short one-liners stay
 * collapsed since the subtitle already shows them in full.
 */
export function memoryDefaultExpanded(args: unknown): boolean {
  const a = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
  const memory = typeof a.memory === "string" ? a.memory : "";
  return memory.includes("\n") || memory.length > 100;
}
