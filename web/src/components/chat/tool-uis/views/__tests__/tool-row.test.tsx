import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ToolRow } from "../tool-row";

// Make framer-motion animations synchronous and skip AnimatePresence wrapping.
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

describe("ToolRow", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it("renders header and body slots", () => {
    render(
      <ToolRow status={{ type: "complete" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>The Title</ToolRow.Title>
        </ToolRow.Header>
        <ToolRow.Body isExpanded>body content</ToolRow.Body>
      </ToolRow>,
    );
    expect(screen.getByText("The Title")).toBeInTheDocument();
    expect(screen.getByText("body content")).toBeInTheDocument();
  });

  it("omits the subtitle dash when children are empty", () => {
    const { container } = render(
      <ToolRow status={{ type: "complete" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
          <ToolRow.Subtitle>{null}</ToolRow.Subtitle>
        </ToolRow.Header>
      </ToolRow>,
    );
    expect(container.textContent).not.toContain("—");
  });

  it("renders the subtitle dash with content", () => {
    render(
      <ToolRow status={{ type: "complete" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
          <ToolRow.Subtitle>hello</ToolRow.Subtitle>
        </ToolRow.Header>
      </ToolRow>,
    );
    expect(screen.getByText(/— hello/)).toBeInTheDocument();
  });

  it("calls onToggle when expandable", () => {
    const onToggle = vi.fn();
    render(
      <ToolRow status={{ type: "complete" }}>
        <ToolRow.Header onToggle={onToggle} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
        </ToolRow.Header>
      </ToolRow>,
    );
    fireEvent.click(screen.getByRole("button"));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it("disables header and hides chevron when expandable=false", () => {
    const onToggle = vi.fn();
    render(
      <ToolRow status={{ type: "complete" }} expandable={false}>
        <ToolRow.Header onToggle={onToggle} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
        </ToolRow.Header>
      </ToolRow>,
    );
    const button = screen.getByRole("button");
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(onToggle).not.toHaveBeenCalled();
  });

  it("applies line-through to header when cancelled", () => {
    render(
      <ToolRow status={{ type: "incomplete", reason: "cancelled" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
        </ToolRow.Header>
      </ToolRow>,
    );
    expect(screen.getByRole("button")).toHaveClass("line-through");
  });

  it("renders DefaultErrorBlock when errored and no <ToolRow.Error> slot", () => {
    render(
      <ToolRow status={{ type: "incomplete", reason: "error", error: "boom" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
        </ToolRow.Header>
        <ToolRow.Body isExpanded>regular body</ToolRow.Body>
      </ToolRow>,
    );
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
    expect(screen.queryByText("regular body")).not.toBeInTheDocument();
  });

  it("renders custom <ToolRow.Error> content instead of DefaultErrorBlock when errored", () => {
    render(
      <ToolRow status={{ type: "incomplete", reason: "error", error: "boom" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
        </ToolRow.Header>
        <ToolRow.Body isExpanded>regular body</ToolRow.Body>
        <ToolRow.Error>
          <span>custom error display</span>
        </ToolRow.Error>
      </ToolRow>,
    );
    expect(screen.getByText("custom error display")).toBeInTheDocument();
    expect(screen.queryByText("Failed")).not.toBeInTheDocument();
    expect(screen.queryByText("regular body")).not.toBeInTheDocument();
  });

  it("does NOT render <ToolRow.Error> content when not errored", () => {
    render(
      <ToolRow status={{ type: "complete" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
        </ToolRow.Header>
        <ToolRow.Body isExpanded>regular body</ToolRow.Body>
        <ToolRow.Error>
          <span>custom error display</span>
        </ToolRow.Error>
      </ToolRow>,
    );
    expect(screen.getByText("regular body")).toBeInTheDocument();
    expect(screen.queryByText("custom error display")).not.toBeInTheDocument();
  });

  it("warns in dev when Header slot is missing", () => {
    render(
      <ToolRow status={{ type: "complete" }}>
        <ToolRow.Body isExpanded>body</ToolRow.Body>
      </ToolRow>,
    );
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("missing <ToolRow.Header>"),
    );
  });

  it("renders cancelled reason and dimmed body when cancelled with reason", () => {
    render(
      <ToolRow status={{ type: "incomplete", reason: "cancelled", error: "user stopped" }}>
        <ToolRow.Header onToggle={() => {}} isExpanded>
          <ToolRow.Title>X</ToolRow.Title>
        </ToolRow.Header>
        <ToolRow.Body isExpanded>regular body</ToolRow.Body>
      </ToolRow>,
    );
    expect(screen.getByText("Cancelled")).toBeInTheDocument();
    expect(screen.getByText("user stopped")).toBeInTheDocument();
    expect(screen.getByText("regular body")).toBeInTheDocument();
  });
});
