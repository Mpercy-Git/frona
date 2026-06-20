import { vi } from "vitest";
import type { ToolViewProps } from "../types";

export function mkProps(overrides: Partial<ToolViewProps> = {}): ToolViewProps {
  const base = {
    toolName: "demo_tool",
    toolCallId: "tc-1",
    args: {},
    argsText: "",
    result: undefined,
    status: { type: "complete" },
    isExpanded: true,
    onToggle: vi.fn(),
    // ToolCallMessagePartProps requires these; tests don't use them.
    addResult: vi.fn(),
    resume: vi.fn(),
    // MessagePartState fields — also unused in tests but required by the type.
    type: "tool-call",
  };
  return { ...base, ...overrides } as unknown as ToolViewProps;
}
