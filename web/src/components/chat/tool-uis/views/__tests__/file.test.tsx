import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { FileView } from "../file";
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
    lineNumbers,
  }: {
    code: string;
    language?: string;
    lineNumbers?: boolean;
  }) => (
    <pre
      data-testid="code-block"
      data-lang={language ?? ""}
      data-line-numbers={lineNumbers ? "1" : "0"}
    >
      {code}
    </pre>
  ),
}));

describe("FileView — read", () => {
  it("uses Read title, path subtitle, and infers language from extension", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "read",
          args: { path: "src/foo.ts" },
          result: "const x = 1\n",
        })}
      />,
    );
    expect(screen.getByText("Read")).toBeInTheDocument();
    expect(screen.getByText(/— src\/foo\.ts/)).toBeInTheDocument();
    const block = screen.getByTestId("code-block");
    expect(block).toHaveAttribute("data-lang", "ts");
    expect(block).toHaveAttribute("data-line-numbers", "1");
    expect(block).toHaveTextContent("const x = 1");
  });

  it("infers python from .py extension", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "read",
          args: { path: "a.py" },
          result: "x = 1",
        })}
      />,
    );
    expect(screen.getByTestId("code-block")).toHaveAttribute("data-lang", "python");
  });

  it("infers docker for Dockerfile", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "read",
          args: { path: "Dockerfile" },
          result: "FROM alpine",
        })}
      />,
    );
    expect(screen.getByTestId("code-block")).toHaveAttribute("data-lang", "docker");
  });

  it("falls back to text for unknown extensions", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "read",
          args: { path: "weird.xyz" },
          result: "stuff",
        })}
      />,
    );
    expect(screen.getByTestId("code-block")).toHaveAttribute("data-lang", "text");
  });
});

describe("FileView — write", () => {
  it("renders args.content with inferred language", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "write",
          args: { path: "x.json", content: '{"a":1}' },
        })}
      />,
    );
    expect(screen.getByText("Write")).toBeInTheDocument();
    const block = screen.getByTestId("code-block");
    expect(block).toHaveAttribute("data-lang", "json");
    expect(block).toHaveTextContent('{"a":1}');
  });
});

describe("FileView — edit", () => {
  it("splits summary and surrounding context", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "edit",
          args: { path: "f.ts" },
          result: "Edited 1 occurrence\nSurrounding context:\nconst x = 2",
        })}
      />,
    );
    expect(screen.getByText("Edited 1 occurrence")).toBeInTheDocument();
    expect(screen.getByTestId("code-block")).toHaveTextContent("const x = 2");
  });

  it("renders raw result when marker is absent", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "edit",
          args: { path: "f.ts" },
          result: "no marker here",
        })}
      />,
    );
    expect(screen.getByText("no marker here")).toBeInTheDocument();
    expect(screen.queryByTestId("code-block")).not.toBeInTheDocument();
  });
});

describe("FileView — glob", () => {
  it("renders 'pattern' as subtitle when path is '.'", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "glob",
          args: { pattern: "*.ts", path: "." },
          result: "a.ts\nb.ts",
        })}
      />,
    );
    expect(screen.getByText("Glob")).toBeInTheDocument();
    expect(screen.getByText(/— \*\.ts/)).toBeInTheDocument();
  });

  it("renders 'pattern in scope' when path is something else", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "glob",
          args: { pattern: "*.ts", path: "src" },
          result: "src/a.ts",
        })}
      />,
    );
    expect(screen.getByText(/— \*\.ts in src/)).toBeInTheDocument();
  });

  it("renders matched paths line by line", () => {
    // Glob highlights literal substrings of the pattern (".ts" here), so the
    // line text is split across <span>s — assert via textContent.
    const { container } = render(
      <FileView
        {...mkProps({
          toolName: "glob",
          args: { pattern: "*.ts" },
          result: "a.ts\nb.ts",
        })}
      />,
    );
    const lines = container.querySelectorAll("pre > div");
    const texts = Array.from(lines).map((el) => el.textContent);
    expect(texts).toEqual(expect.arrayContaining(["a.ts", "b.ts"]));
  });
});

describe("FileView — grep", () => {
  it("renders file:line:rest with dimmed location and highlighted match", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "grep",
          args: { pattern: "foo" },
          result: "src/a.ts:10:something foo bar",
        })}
      />,
    );
    expect(screen.getByText("src/a.ts:10")).toBeInTheDocument();
    expect(screen.getByText("foo")).toBeInTheDocument();
  });

  it("does not throw on invalid regex pattern", () => {
    expect(() =>
      render(
        <FileView
          {...mkProps({
            toolName: "grep",
            args: { pattern: "[" },
            result: "src/a.ts:1:something",
          })}
        />,
      ),
    ).not.toThrow();
  });

  it("renders truncation marker as italic tail", () => {
    render(
      <FileView
        {...mkProps({
          toolName: "grep",
          args: { pattern: "x" },
          result: "src/a.ts:1:hi\n\n[truncated 5 more results]",
        })}
      />,
    );
    expect(screen.getByText("[truncated 5 more results]")).toBeInTheDocument();
  });
});
