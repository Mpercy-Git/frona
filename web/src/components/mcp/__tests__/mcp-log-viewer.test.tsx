import { describe, it, expect } from "vitest";
import { classifyLine, parseSseChunk } from "../mcp-log-viewer";

describe("parseSseChunk", () => {
  it("returns complete data frames and keeps the unterminated remainder", () => {
    const { frames, rest } = parseSseChunk("data: first\n\ndata: second\n\ndata: thi");
    expect(frames).toEqual([
      { kind: "line", text: "first" },
      { kind: "line", text: "second" },
    ]);
    expect(rest).toBe("data: thi");
  });

  it("turns a reset event into a reset frame", () => {
    const { frames } = parseSseChunk("data: old\n\nevent: reset\ndata: \n\ndata: new\n\n");
    expect(frames).toEqual([
      { kind: "line", text: "old" },
      { kind: "reset" },
      { kind: "line", text: "new" },
    ]);
  });

  it("ignores keep-alive comments", () => {
    const { frames } = parseSseChunk(":\n\ndata: x\n\n");
    expect(frames).toEqual([{ kind: "line", text: "x" }]);
  });
});

describe("classifyLine", () => {
  it("guesses a level from the words in a free-form line", () => {
    expect(classifyLine("Error: connect ECONNREFUSED")).toBe("error");
    expect(classifyLine("Traceback (most recent call last):")).toBe("error");
    expect(classifyLine("(node:12) Warning: something deprecated")).toBe("warn");
    expect(classifyLine("Server listening on stdio")).toBe("info");
    expect(classifyLine("terrible")).toBe("info");
  });
});
