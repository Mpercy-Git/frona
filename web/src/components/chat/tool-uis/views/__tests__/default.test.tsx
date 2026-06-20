import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { DefaultView } from "../default";
import { mkProps } from "./helpers";

vi.mock("@/components/ui/code-block", () => ({
  CodeBlock: ({ code, language }: { code: string; language?: string }) => (
    <pre data-testid="code-block" data-lang={language ?? ""}>
      {code}
    </pre>
  ),
}));

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

describe("DefaultView", () => {
  it("uses TOOL_DISPLAY_NAMES for known tool names", () => {
    render(<DefaultView {...mkProps({ toolName: "web_fetch" })} />);
    expect(screen.getByText("Web Fetch")).toBeInTheDocument();
  });

  it("title-cases unknown tool names with underscores", () => {
    render(<DefaultView {...mkProps({ toolName: "foo_bar_baz" })} />);
    expect(screen.getByText("Foo Bar Baz")).toBeInTheDocument();
  });

  it("unwraps MCP-prefixed names and title-cases the tail", () => {
    render(<DefaultView {...mkProps({ toolName: "mcp__github__create_pr" })} />);
    expect(screen.getByText("Create Pr")).toBeInTheDocument();
  });

  it("renders args.description as subtitle when different from toolName", () => {
    render(
      <DefaultView
        {...mkProps({ toolName: "demo_tool", args: { description: "doing stuff" } })}
      />,
    );
    expect(screen.getByText(/— doing stuff/)).toBeInTheDocument();
  });

  it("suppresses subtitle when args.description equals toolName", () => {
    const { container } = render(
      <DefaultView
        {...mkProps({ toolName: "demo_tool", args: { description: "demo_tool" } })}
      />,
    );
    expect(container.textContent).not.toContain("—");
  });

  it("renders argsText as syntax-highlighted JSON in the body", () => {
    render(
      <DefaultView
        {...mkProps({
          argsText: '{"q":"x"}',
          result: { ok: true },
        })}
      />,
    );
    const block = screen.getByTestId("code-block");
    expect(block).toHaveAttribute("data-lang", "json");
    expect(block).toHaveTextContent('"q": "x"');
    expect(screen.getByText(/"ok": true/)).toBeInTheDocument();
    expect(screen.getByText("Result:")).toBeInTheDocument();
  });

  it("strips the outer braces from JSON-object args", () => {
    render(
      <DefaultView
        {...mkProps({
          argsText: '{"a":1,"b":"two"}',
        })}
      />,
    );
    const block = screen.getByTestId("code-block");
    expect(block.textContent).not.toMatch(/^\{/);
    expect(block.textContent).not.toMatch(/\}$/);
    expect(block.textContent).toContain('"a": 1');
    expect(block.textContent).toContain('"b": "two"');
  });

  it("keeps brackets for non-object JSON (arrays, primitives)", () => {
    render(
      <DefaultView
        {...mkProps({
          argsText: "[1,2,3]",
        })}
      />,
    );
    const block = screen.getByTestId("code-block");
    expect(block.textContent).toContain("[");
    expect(block.textContent).toContain("]");
    expect(block.textContent).toContain("1");
  });

  it("renders no args block when args is an empty object", () => {
    render(
      <DefaultView
        {...mkProps({
          argsText: "{}",
        })}
      />,
    );
    expect(screen.queryByTestId("code-block")).not.toBeInTheDocument();
  });

  it("falls back to plain <pre> when argsText is not valid JSON", () => {
    render(
      <DefaultView
        {...mkProps({
          argsText: "not actually json",
        })}
      />,
    );
    expect(screen.queryByTestId("code-block")).not.toBeInTheDocument();
    expect(screen.getByText("not actually json")).toBeInTheDocument();
  });

  it("renders a string result verbatim (no JSON.stringify)", () => {
    render(<DefaultView {...mkProps({ result: "plain string" })} />);
    expect(screen.getByText("plain string")).toBeInTheDocument();
  });

  it("omits the result block when result is undefined", () => {
    render(<DefaultView {...mkProps({ argsText: "{}", result: undefined })} />);
    expect(screen.queryByText("Result:")).not.toBeInTheDocument();
  });

  it("on error, shows args and the error message (no result block)", () => {
    render(
      <DefaultView
        {...mkProps({
          toolName: "web_fetch",
          argsText: '{"url":"https://broken.example"}',
          result: undefined,
          status: { type: "incomplete", reason: "error", error: "Boom" },
        })}
      />,
    );
    // Args still visible on error — rendered via the JSON CodeBlock.
    expect(screen.getByTestId("code-block")).toHaveTextContent(
      '"url": "https://broken.example"',
    );
    // Custom error block (not the bare DefaultErrorBlock).
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Boom")).toBeInTheDocument();
    // Result block should not appear (no result).
    expect(screen.queryByText("Result:")).not.toBeInTheDocument();
  });

  it("renders default error block when status is errored", () => {
    render(
      <DefaultView
        {...mkProps({
          status: { type: "incomplete", reason: "error", error: "kaboom" },
        })}
      />,
    );
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("kaboom")).toBeInTheDocument();
  });
});
