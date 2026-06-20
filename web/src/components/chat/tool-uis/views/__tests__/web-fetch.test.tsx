import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { WebFetchView } from "../web-fetch";
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
  CodeBlock: ({ code }: { code: string }) => <pre data-testid="code-block">{code}</pre>,
}));

describe("WebFetchView", () => {
  it("renders Web Fetch title and host+path subtitle", () => {
    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "https://example.com/docs/guide?ref=foo" },
          result: "# Hello world",
        })}
      />,
    );
    expect(screen.getByText("Web Fetch")).toBeInTheDocument();
    expect(
      screen.getByText("— example.com/docs/guide?ref=foo"),
    ).toBeInTheDocument();
  });

  it("renders the URL as a clickable link in the body", () => {
    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "https://example.com/x" },
          result: "page content",
        })}
      />,
    );
    const link = screen.getByRole("link", { name: "https://example.com/x" });
    expect(link).toHaveAttribute("href", "https://example.com/x");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", expect.stringContaining("noopener"));
  });

  it("clicking the URL opens it in a new popup window", () => {
    const open = vi.fn();
    Object.assign(window, { open });

    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "https://example.com/x" },
          result: "x",
        })}
      />,
    );
    fireEvent.click(screen.getByRole("link", { name: "https://example.com/x" }));
    expect(open).toHaveBeenCalledWith(
      "https://example.com/x",
      "_blank",
      expect.stringContaining("width="),
    );
  });

  it("copy button writes the URL to the clipboard", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "https://example.com/path" },
          result: "x",
        })}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Copy URL" }));
    expect(writeText).toHaveBeenCalledWith("https://example.com/path");
  });

  it("renders markdown result via ReactMarkdown", () => {
    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "https://example.com" },
          result: "# Page heading\n\nSome **body** text.",
        })}
      />,
    );
    expect(screen.getByRole("heading", { name: "Page heading" })).toBeInTheDocument();
    // The bold text gets wrapped in <strong>
    expect(screen.getByText("body")).toBeInTheDocument();
  });

  it("falls back to raw URL when the URL doesn't parse", () => {
    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "not-a-real-url" },
          result: "content",
        })}
      />,
    );
    expect(screen.getByText("— not-a-real-url")).toBeInTheDocument();
  });

  it("trims trailing slash-only paths from the subtitle", () => {
    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "https://example.com/" },
          result: "x",
        })}
      />,
    );
    expect(screen.getByText("— example.com")).toBeInTheDocument();
  });

  it("renders a custom 'Fetch failed' block with the URL still shown on error", () => {
    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: { url: "https://broken.example/" },
          result: undefined,
          status: {
            type: "incomplete",
            reason: "error",
            error: "Browser error: tool get_markdown: Readability.parse() returned null",
          },
        })}
      />,
    );
    expect(screen.getByText("Fetch failed")).toBeInTheDocument();
    expect(
      screen.getByText(/Browser error: tool get_markdown/),
    ).toBeInTheDocument();
    // URL block remains visible above the error.
    expect(
      screen.getByRole("link", { name: "https://broken.example/" }),
    ).toBeInTheDocument();
    // The default generic "Failed" header is NOT used (custom "Fetch failed" instead).
    expect(screen.queryByText("Failed")).not.toBeInTheDocument();
  });

  it("disables expansion when there's no URL and no result", () => {
    render(
      <WebFetchView
        {...mkProps({
          toolName: "web_fetch",
          args: {},
          result: undefined,
        })}
      />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });
});
