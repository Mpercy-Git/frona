import { describe, it, expect } from "vitest";
import { renderMessageBody, renderResultMarkdown } from "../task-result-render";
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
  it("appends a Sources section when citations are present", () => {
    const msg = taskCompletionMessage({
      content: "Found the answer.",
      citations: [
        { title: "Rust Programming", url: "https://rust-lang.org" },
        { url: "https://doc.rust-lang.org/book/" },
      ],
    });

    const body = renderMessageBody(msg);
    expect(body).toBe(
      "Found the answer.\n\n**Sources**\n" +
        "- [Rust Programming](https://rust-lang.org)\n" +
        "- [https://doc.rust-lang.org/book/](https://doc.rust-lang.org/book/)",
    );
  });

  it("omits the Sources section when there are no citations", () => {
    const msg = taskCompletionMessage({ content: "Found the answer." });
    expect(renderMessageBody(msg)).toBe("Found the answer.");
  });

  it("does not attach sources to a suppressed (null) complex-schema result", () => {
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
    expect(renderMessageBody(msg)).toBe("");
  });

  it("attaches citations to a schema-rendered summary field", () => {
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
    expect(renderMessageBody(msg)).toBe(
      "Here you go.\n\n**Sources**\n- [Example](https://example.com)",
    );
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

describe("renderResultMarkdown (sanity, unaffected by citations)", () => {
  it("still renders scalar values directly", () => {
    expect(renderResultMarkdown({ type: "string" }, "hello")).toBe("hello");
  });
});
