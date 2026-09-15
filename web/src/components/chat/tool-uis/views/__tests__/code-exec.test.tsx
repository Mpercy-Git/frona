import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { NodeView } from "../node";
import { execTranscript } from "../code-exec";
import { PythonView } from "../python";
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

vi.mock("@/components/ui/code-block", () => ({
  CodeBlock: ({
    code,
    language,
    wrap,
    lineNumbers,
    copyText,
  }: {
    code: string;
    language?: string;
    wrap?: boolean;
    lineNumbers?: boolean;
    copyText?: string;
  }) => (
    <pre
      data-testid="code-block"
      data-lang={language ?? ""}
      data-wrap={wrap ? "1" : "0"}
      data-line-numbers={lineNumbers ? "1" : "0"}
      data-copy-text={copyText ?? ""}
    >
      {code}
    </pre>
  ),
}));

describe("PythonView", () => {
  it("renders 'Python' title and a python-highlighted CodeBlock", () => {
    render(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: { code: "print('hello')" },
          result: "hello",
        })}
      />,
    );
    expect(screen.getByText("Python")).toBeInTheDocument();
    const block = screen.getAllByTestId("code-block")[0];
    expect(block).toHaveAttribute("data-lang", "python");
    expect(block).toHaveAttribute("data-wrap", "0");
    expect(block).toHaveAttribute("data-line-numbers", "1");
    expect(block).toHaveTextContent("print('hello')");
  });

  it("uses args.description as subtitle when set", () => {
    render(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: {
            code: "import pandas as pd\ndf = pd.read_csv('x.csv')",
            description: "Load the CSV into a DataFrame",
          },
          result: "ok",
        })}
      />,
    );
    expect(
      screen.getByText("— Load the CSV into a DataFrame"),
    ).toBeInTheDocument();
  });

  it("omits the subtitle when description is missing or equals the tool name", () => {
    const { container, rerender } = render(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: { code: "x = 1" },
          result: "ok",
        })}
      />,
    );
    expect(container.textContent).not.toContain("—");

    rerender(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: { code: "x = 1", description: "python" },
          result: "ok",
        })}
      />,
    );
    expect(container.textContent).not.toContain("—");
  });

  it("renders 'Python failed' on error", () => {
    render(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: { code: "raise ValueError('x')" },
          status: { type: "incomplete", reason: "error", error: "ValueError: x" },
        })}
      />,
    );
    expect(screen.getByText("Python failed")).toBeInTheDocument();
  });

  it("renders stdout/stderr below the code block", () => {
    render(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: { code: "print('a')" },
          result: "a\n",
        })}
      />,
    );
    expect(screen.getByText(/^a$/m)).toBeInTheDocument();
  });

  it("surfaces sandbox-deny events from python output", () => {
    const result =
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1.2.3.4!80"}';
    render(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: { code: "import urllib.request; urllib.request.urlopen('http://x')" },
          result,
        })}
      />,
    );
    expect(screen.getByText(/Sandbox deny/)).toBeInTheDocument();
    expect(screen.getByText("1.2.3.4:80")).toBeInTheDocument();
  });
});

describe("NodeView", () => {
  it("renders 'Node' title and a javascript-highlighted CodeBlock", () => {
    render(
      <NodeView
        {...mkProps({
          toolName: "node",
          args: { code: "console.log('hi')" },
          result: "hi",
        })}
      />,
    );
    expect(screen.getByText("Node")).toBeInTheDocument();
    const block = screen.getAllByTestId("code-block")[0];
    expect(block).toHaveAttribute("data-lang", "javascript");
    expect(block).toHaveAttribute("data-wrap", "0");
    expect(block).toHaveAttribute("data-line-numbers", "1");
    expect(block).toHaveTextContent("console.log('hi')");
  });

  it("uses args.description as subtitle when set", () => {
    render(
      <NodeView
        {...mkProps({
          toolName: "node",
          args: {
            code: "const fs = require('fs');\nfs.readFileSync('x')",
            description: "Read the file synchronously",
          },
          result: "",
        })}
      />,
    );
    expect(
      screen.getByText("— Read the file synchronously"),
    ).toBeInTheDocument();
  });

  it("renders 'Node failed' on error", () => {
    render(
      <NodeView
        {...mkProps({
          toolName: "node",
          args: { code: "throw new Error('x')" },
          status: { type: "incomplete", reason: "error", error: "Error: x" },
        })}
      />,
    );
    expect(screen.getByText("Node failed")).toBeInTheDocument();
  });

  it("disables expansion when there's no code and no result", () => {
    render(
      <NodeView {...mkProps({ toolName: "node", args: {}, result: undefined })} />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });
});

describe("execTranscript", () => {
  it("joins the command and its output so a copy carries both", () => {
    expect(execTranscript("ls -la", "total 0", null)).toBe("ls -la\n\ntotal 0");
  });

  it("keeps the raw output rather than what the sandbox summary renders", () => {
    // The panel lifts syd events out of `remainingText` and draws them as a
    // grouped block, so the parsed view never shows the original line. A paste
    // of a denial is only useful with that line intact.
    const raw = '{"cap":"stat","act":"deny","sys":"newfstatat","path":"/"}';
    expect(execTranscript("mcpctl list", raw, null)).toContain('"path":"/"');
  });

  it("appends an error that says something the result doesn't", () => {
    expect(execTranscript("bad", "stdout", "Error: boom")).toBe(
      "bad\n\nstdout\n\nError: boom",
    );
  });

  it("does not repeat an error identical to the result", () => {
    expect(execTranscript("bad", "Error: boom", "Error: boom")).toBe(
      "bad\n\nError: boom",
    );
  });

  it("omits empty sections rather than leaving blank runs", () => {
    expect(execTranscript("ls", "", null)).toBe("ls");
    expect(execTranscript("", "orphan output", null)).toBe("orphan output");
  });
});

describe("copy affordance", () => {
  it("hands the code block the command and output together", () => {
    render(
      <PythonView
        {...mkProps({
          toolName: "python",
          args: { code: "print('hello')" },
          result: "hello",
        })}
      />,
    );
    expect(screen.getAllByTestId("code-block")[0]).toHaveAttribute(
      "data-copy-text",
      "print('hello')\n\nhello",
    );
  });

  it("includes the output on the failure path too", () => {
    render(
      <NodeView
        {...mkProps({
          toolName: "node",
          args: { code: "throw new Error('x')" },
          status: { type: "incomplete", reason: "error", error: "Error: x" },
        })}
      />,
    );
    expect(screen.getAllByTestId("code-block")[0]).toHaveAttribute(
      "data-copy-text",
      "throw new Error('x')\n\nError: x",
    );
  });
});
