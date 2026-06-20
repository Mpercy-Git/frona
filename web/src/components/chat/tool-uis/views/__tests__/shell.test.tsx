import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ShellView } from "../shell";
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
  }: {
    code: string;
    language?: string;
    wrap?: boolean;
    lineNumbers?: boolean;
  }) => (
    <pre
      data-testid="code-block"
      data-lang={language ?? ""}
      data-wrap={wrap ? "1" : "0"}
      data-line-numbers={lineNumbers ? "1" : "0"}
    >
      {code}
    </pre>
  ),
}));

describe("ShellView", () => {
  it("renders command-derived title and subtitle (curl + URL)", () => {
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "curl https://example.com" },
        })}
      />,
    );
    expect(screen.getByText("Curl")).toBeInTheDocument();
    expect(screen.getByText(/— https:\/\/example\.com/)).toBeInTheDocument();
  });

  it("falls back to Shell title for unparseable / compound commands", () => {
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "find . | grep foo" },
        })}
      />,
    );
    expect(screen.getByText("Shell")).toBeInTheDocument();
    expect(screen.getByText(/— find \. \| grep foo/)).toBeInTheDocument();
  });

  it("renders command as a bash code block with wrap", () => {
    render(
      <ShellView
        {...mkProps({ toolName: "shell", args: { command: "echo hi" } })}
      />,
    );
    const blocks = screen.getAllByTestId("code-block");
    expect(blocks.length).toBeGreaterThan(0);
    expect(blocks[0]).toHaveAttribute("data-lang", "bash");
    expect(blocks[0]).toHaveAttribute("data-wrap", "1");
    expect(blocks[0]).toHaveTextContent("echo hi");
  });

  it("renders result text as a preformatted block", () => {
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "echo hi" },
          result: "shell-output-line\n",
        })}
      />,
    );
    expect(screen.getByText(/shell-output-line/)).toBeInTheDocument();
  });

  it("renders shell-specific 'Command failed' block on error", () => {
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "false" },
          status: { type: "incomplete", reason: "error", error: "exit 1" },
        })}
      />,
    );
    expect(screen.getByText("Command failed")).toBeInTheDocument();
    expect(screen.getByText("exit 1")).toBeInTheDocument();
    expect(screen.queryByText("Failed")).not.toBeInTheDocument();
  });

  it("renders a sandbox lock icon in the header when sandbox events are present", () => {
    const result =
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1.2.3.4!80"}';
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "curl example.com" },
          result,
        })}
      />,
    );
    expect(
      screen.getByLabelText(/Sandbox denied 1 action/),
    ).toBeInTheDocument();
  });

  it("does NOT render the sandbox icon when there are no events", () => {
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "curl example.com" },
          result: "plain output, no events",
        })}
      />,
    );
    expect(screen.queryByLabelText(/Sandbox denied/)).not.toBeInTheDocument();
  });

  it("renders a SandboxBlock when result contains syd deny events", () => {
    const result =
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1.2.3.4!80","tip":"configure allow/net/connect+1.2.3.4!80"}\n% Total trailing stderr text';
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "curl example.com" },
          result,
        })}
      />,
    );
    expect(screen.getByText(/Sandbox deny/)).toBeInTheDocument();
    expect(screen.getByText("1.2.3.4:80")).toBeInTheDocument();
    expect(screen.getByText("(net/connect)")).toBeInTheDocument();
    expect(screen.getByText(/configure allow\/net\/connect/)).toBeInTheDocument();
    // The non-event line still renders.
    expect(screen.getByText(/% Total trailing stderr text/)).toBeInTheDocument();
  });

  it("renders SandboxBlock even on success (non-fatal denials)", () => {
    const result =
      'normal output\n{"ctx":"access","cap":"file/read","act":"deny","sys":"openat","path":"/etc/shadow"}\nmore output';
    render(
      <ShellView
        {...mkProps({
          toolName: "shell",
          args: { command: "cat /etc/shadow 2>/dev/null || true" },
          result,
          status: { type: "complete" },
        })}
      />,
    );
    expect(screen.getByText(/Sandbox deny/)).toBeInTheDocument();
    expect(screen.getByText("/etc/shadow")).toBeInTheDocument();
  });

  it("hides the chevron and disables expansion when command and result are empty", () => {
    render(
      <ShellView
        {...mkProps({ toolName: "shell", args: {}, result: undefined })}
      />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });
});
