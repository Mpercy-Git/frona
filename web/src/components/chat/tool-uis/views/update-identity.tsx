"use client";

import { ToolRow } from "./tool-row";
import type { ToolView } from "./types";

function parseAttributes(args: unknown): Array<[string, string]> {
  const a = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
  const attrs = a.attributes;
  if (!attrs || typeof attrs !== "object" || Array.isArray(attrs)) return [];
  return Object.entries(attrs as Record<string, unknown>).map(([k, v]) => [
    k,
    typeof v === "string" ? v : JSON.stringify(v),
  ]);
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return s.slice(0, max - 1) + "…";
}

export const UpdateIdentityView: ToolView = ({
  args,
  status,
  isExpanded,
  onToggle,
}) => {
  const entries = parseAttributes(args);
  const sets = entries.filter(([, v]) => v !== "");
  const removes = entries.filter(([, v]) => v === "");

  const subtitle =
    entries.length > 0 ? truncate(entries.map(([k]) => k).join(", "), 80) : null;
  const expandable = entries.length > 0;

  return (
    <ToolRow status={status} expandable={expandable}>
      <ToolRow.Header onToggle={onToggle} isExpanded={isExpanded}>
        <ToolRow.Title>Identity</ToolRow.Title>
        <ToolRow.Subtitle>{subtitle}</ToolRow.Subtitle>
      </ToolRow.Header>
      <ToolRow.Body isExpanded={isExpanded} unstyled>
        <div className="flex flex-col gap-1.5 p-3 text-xs">
          {sets.map(([k, v]) => (
            <div
              key={k}
              className="flex items-baseline gap-2 flex-wrap"
            >
              <span className="font-mono font-medium text-text-secondary">
                {k}
              </span>
              <span className="text-text-tertiary">=</span>
              <span className="text-text-primary whitespace-pre-wrap break-words">
                {v}
              </span>
            </div>
          ))}
          {removes.map(([k]) => (
            <div key={k} className="flex items-center gap-2">
              <span className="font-mono text-text-tertiary line-through">{k}</span>
              <span className="text-text-tertiary italic">removed</span>
            </div>
          ))}
        </div>
      </ToolRow.Body>
    </ToolRow>
  );
};
