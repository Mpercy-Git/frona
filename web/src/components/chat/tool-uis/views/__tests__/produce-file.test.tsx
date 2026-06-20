import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ProduceFileView, formatBytes } from "../produce-file";
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

describe("ProduceFileView", () => {
  it("renders title and path subtitle", () => {
    render(
      <ProduceFileView
        {...mkProps({
          toolName: "produce_file",
          args: { path: "out.txt" },
        })}
      />,
    );
    expect(screen.getByText("Produce File")).toBeInTheDocument();
    expect(screen.getByText(/— out\.txt/)).toBeInTheDocument();
  });

  it("parses string result as JSON and shows filename + size + content_type", () => {
    render(
      <ProduceFileView
        {...mkProps({
          toolName: "produce_file",
          args: { path: "x" },
          result: JSON.stringify({
            filename: "out.bin",
            content_type: "application/octet-stream",
            size_bytes: 2048,
          }),
        })}
      />,
    );
    expect(screen.getByText("out.bin")).toBeInTheDocument();
    expect(screen.getByText("2.0 KB")).toBeInTheDocument();
    expect(screen.getByText("application/octet-stream")).toBeInTheDocument();
  });

  it("accepts object result directly", () => {
    render(
      <ProduceFileView
        {...mkProps({
          toolName: "produce_file",
          args: { path: "x" },
          result: { filename: "a.json", size_bytes: 100, content_type: "application/json" },
        })}
      />,
    );
    expect(screen.getByText("a.json")).toBeInTheDocument();
    expect(screen.getByText("100 B")).toBeInTheDocument();
  });

  it("falls back to args.path for filename when result is missing", () => {
    render(
      <ProduceFileView
        {...mkProps({
          toolName: "produce_file",
          args: { path: "fallback.md" },
        })}
      />,
    );
    expect(screen.getByText("fallback.md")).toBeInTheDocument();
  });
});

describe("formatBytes", () => {
  it.each<[number, string]>([
    [0, "0 B"],
    [1023, "1023 B"],
    [1024, "1.0 KB"],
    [1024 * 1024, "1.0 MB"],
    [1024 * 1024 * 1024, "1.00 GB"],
  ])("formats %i bytes as %s", (input, expected) => {
    expect(formatBytes(input)).toBe(expected);
  });
});
