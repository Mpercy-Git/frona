import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  memoryDefaultExpanded,
  StoreAgentMemoryView,
  StoreUserMemoryView,
} from "../memory";
import { mkProps } from "./helpers";

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy({}, {
    get: () => (props: Record<string, unknown>) => {
      const { children, ...rest } = props as { children?: React.ReactNode } & Record<string, unknown>;
      const filtered: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(rest)) {
        if (k === "initial" || k === "animate" || k === "exit" || k === "transition") continue;
        filtered[k] = v;
      }
      return <div {...filtered}>{children}</div>;
    },
  }),
}));

describe("StoreAgentMemoryView", () => {
  it("uses 'Remember' as title", () => {
    render(
      <StoreAgentMemoryView
        {...mkProps({
          toolName: "store_agent_memory",
          args: { memory: "User prefers TypeScript over JavaScript" },
        })}
      />,
    );
    expect(screen.getByText("Remember")).toBeInTheDocument();
  });

  it("renders the memory text as the subtitle when it fits in one line", () => {
    render(
      <StoreAgentMemoryView
        {...mkProps({
          toolName: "store_agent_memory",
          args: { memory: "User prefers TypeScript" },
        })}
      />,
    );
    expect(screen.getByText("— User prefers TypeScript")).toBeInTheDocument();
  });

  it("truncates the subtitle to 80 chars when memory is long", () => {
    const long = "x".repeat(200);
    const { container } = render(
      <StoreAgentMemoryView
        {...mkProps({
          toolName: "store_agent_memory",
          args: { memory: long },
        })}
      />,
    );
    const subtitle = container.querySelector("span.font-normal");
    expect(subtitle).not.toBeNull();
    const text = subtitle!.textContent ?? "";
    expect(text).toContain("…");
    // " — " (3) + truncated text capped at 80 chars
    expect(text.length).toBeLessThanOrEqual(83);
  });

  it("disables expansion when memory fits entirely in the subtitle", () => {
    render(
      <StoreAgentMemoryView
        {...mkProps({
          toolName: "store_agent_memory",
          args: { memory: "Short" },
        })}
      />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });

  it("renders the full memory in the body for multi-line input", () => {
    render(
      <StoreAgentMemoryView
        {...mkProps({
          toolName: "store_agent_memory",
          args: { memory: "Line 1\nLine 2\nLine 3" },
        })}
      />,
    );
    // Subtitle is first line; body contains the full text.
    expect(screen.getByText("— Line 1")).toBeInTheDocument();
    expect(screen.getByText(/Line 1\s*Line 2\s*Line 3/)).toBeInTheDocument();
  });
});

describe("memoryDefaultExpanded", () => {
  it("returns false for short single-line memory", () => {
    expect(memoryDefaultExpanded({ memory: "Short note" })).toBe(false);
  });

  it("returns true for multi-line memory", () => {
    expect(memoryDefaultExpanded({ memory: "Line 1\nLine 2" })).toBe(true);
  });

  it("returns true for long memory (> 100 chars)", () => {
    expect(memoryDefaultExpanded({ memory: "x".repeat(101) })).toBe(true);
  });

  it("returns false for empty / missing memory", () => {
    expect(memoryDefaultExpanded({})).toBe(false);
    expect(memoryDefaultExpanded(null)).toBe(false);
    expect(memoryDefaultExpanded({ memory: "" })).toBe(false);
  });
});

describe("StoreUserMemoryView", () => {
  it("uses 'Remember about user' as title", () => {
    render(
      <StoreUserMemoryView
        {...mkProps({
          toolName: "store_user_memory",
          args: { memory: "Lives in PDT timezone" },
        })}
      />,
    );
    expect(screen.getByText("Remember about user")).toBeInTheDocument();
    expect(screen.getByText("— Lives in PDT timezone")).toBeInTheDocument();
  });
});
