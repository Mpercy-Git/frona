import { describe, it, expect } from "vitest";
import { renderMessageBody, renderResultMarkdown, getCitations } from "../task-result-render";
import type { MessageResponse } from "../types";

function taskCompletionMessage(
  overrides: Partial<MessageResponse> & {
    content: string;
    schema?: Record<string, unknown>;
    citations?: { title?: string; url: string }[];
  },
): MessageResponse {
  const { schema, citations, ...rest } = overrides;
  return {
    id: "m1",
    chat_id: "c1",
    role: "taskcompletion",
    created_at: new Date().toISOString(),
    event: {
      type: "TaskCompletion",
      data: {
        task_id: "t1",
        chat_id: null,
        status: "completed",
        schema,
        citations,
      },
    },
    ...rest,
  };
}

describe("renderMessageBody: citations", () => {
  it("does not include citations in the rendered body", () => {
    const msg = taskCompletionMessage({
      content: "Found the answer.",
      citations: [
        { title: "Rust Programming", url: "https://rust-lang.org" },
        { url: "https://doc.rust-lang.org/book/" },
      ],
    });

    expect(renderMessageBody(msg)).toBe("Found the answer.");
  });

  it("renders body as-is when there are no citations", () => {
    const msg = taskCompletionMessage({ content: "Found the answer." });
    expect(renderMessageBody(msg)).toBe("Found the answer.");
  });

  it("non-TaskCompletion messages render raw content, ignoring citations", () => {
    const msg: MessageResponse = {
      id: "m2",
      chat_id: "c1",
      role: "agent",
      content: "hello",
      created_at: new Date().toISOString(),
    };
    expect(renderMessageBody(msg)).toBe("hello");
  });
});

describe("getCitations", () => {
  it("returns citations when the body renders non-empty text", () => {
    const msg = taskCompletionMessage({
      content: "Found the answer.",
      citations: [
        { title: "Rust Programming", url: "https://rust-lang.org" },
        { url: "https://doc.rust-lang.org/book/" },
      ],
    });
    expect(getCitations(msg)).toEqual([
      { title: "Rust Programming", url: "https://rust-lang.org" },
      { url: "https://doc.rust-lang.org/book/" },
    ]);
  });

  it("returns [] when there are no citations", () => {
    const msg = taskCompletionMessage({ content: "Found the answer." });
    expect(getCitations(msg)).toEqual([]);
  });

  it("returns [] for a suppressed (null) complex-schema result even with citations", () => {
    const schema = {
      type: "object",
      properties: {
        summary: { type: "string" },
        detail: { type: "object", properties: { x: { type: "string" } } },
      },
    };
    const msg = taskCompletionMessage({
      content: "null",
      schema,
      citations: [{ url: "https://example.com" }],
    });
    expect(getCitations(msg)).toEqual([]);
  });

  it("returns citations for a schema-rendered summary field", () => {
    const schema = {
      type: "object",
      properties: {
        summary: { type: "string" },
        detail: { type: "object", properties: { x: { type: "string" } } },
      },
    };
    const msg = taskCompletionMessage({
      content: JSON.stringify({ summary: "Here you go.", detail: { x: "y" } }),
      schema,
      citations: [{ title: "Example", url: "https://example.com" }],
    });
    expect(getCitations(msg)).toEqual([{ title: "Example", url: "https://example.com" }]);
  });

  it("returns [] for non-TaskCompletion messages", () => {
    const msg: MessageResponse = {
      id: "m2",
      chat_id: "c1",
      role: "agent",
      content: "hello",
      created_at: new Date().toISOString(),
    };
    expect(getCitations(msg)).toEqual([]);
  });
});

describe("renderResultMarkdown (sanity, unaffected by citations)", () => {
  it("still renders scalar values directly", () => {
    expect(renderResultMarkdown({ type: "string" }, "hello")).toBe("hello");
  });
});
