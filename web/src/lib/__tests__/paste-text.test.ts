import { describe, it, expect } from "vitest";

import {
  getClipboardText,
  htmlToPlainText,
  normalizePastedText,
  resolvePaste,
  type ClipboardPayload,
} from "../paste-text";

/** Minimal stand-in for the clipboard of a paste event. */
function clipboard(
  types: Record<string, string>,
  files: File[] = [],
): ClipboardPayload {
  return {
    getData: (type: string) => types[type] ?? "",
    files: files as unknown as FileList,
  };
}

describe("normalizePastedText", () => {
  it("unifies line endings", () => {
    // Lexical only tokenizes \n and \r\n: a bare \r (spreadsheets, PDFs, older
    // Mac apps) would otherwise land in the message as an invisible control
    // character and the lines would run together.
    expect(normalizePastedText("a\r\nb\rc\u2028d\u2029e\u0085f")).toBe(
      "a\nb\nc\nd\ne\nf",
    );
  });

  it("turns lookalike spaces into real spaces", () => {
    expect(normalizePastedText("Hello\u00a0world\u2009again\u3000here")).toBe(
      "Hello world again here",
    );
  });

  it("strips invisible characters", () => {
    expect(normalizePastedText("\ufeff/dep\u200bloy soft\u00adhyphen")).toBe(
      "/deploy softhyphen",
    );
    expect(normalizePastedText("word\u2060joiner")).toBe("wordjoiner");
  });

  // ZWNJ and ZWJ sit in the same Unicode block as the zero-width space above,
  // and stripping the block wholesale silently rewrote the user's words.
  it("keeps the zero-width joiners, which carry meaning", () => {
    // ZWNJ is orthographic: \u0645\u064a\u200c\u0631\u0648\u0645 ("I go") is not \u0645\u064a\u0631\u0648\u0645.
    expect(normalizePastedText("\u0645\u06cc\u200c\u0631\u0648\u0645")).toBe("\u0645\u06cc\u200c\u0631\u0648\u0645");
    // ZWJ is what makes one emoji out of several code points.
    expect(normalizePastedText("\ud83d\udc69\u200d\ud83d\udcbb \ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc67")).toBe(
      "\ud83d\udc69\u200d\ud83d\udcbb \ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc67",
    );
    // A joiner still survives next to an invisible that does get stripped.
    expect(normalizePastedText("\ufeff\u0645\u06cc\u200c\u0631\u0648\u0645")).toBe("\u0645\u06cc\u200c\u0631\u0648\u0645");
  });

  it("drops stray control characters but keeps tabs and newlines", () => {
    expect(normalizePastedText("col\tone\nrow\u0007two\u0000")).toBe(
      "col\tone\nrowtwo",
    );
  });

  it("leaves visible punctuation alone", () => {
    const text = "“Smart quotes” — em dashes … and emoji 🎉";
    expect(normalizePastedText(text)).toBe(text);
  });

  it("is a no-op for text that is already clean", () => {
    expect(normalizePastedText("plain text\nsecond line")).toBe(
      "plain text\nsecond line",
    );
  });
});

describe("htmlToPlainText", () => {
  it("breaks lines on block elements and <br>", () => {
    // Paragraphs keep the blank line between them, as the browser renders
    // them and as markdown needs them; a <br> is a single break.
    expect(htmlToPlainText("<p>one</p><p>two<br>three</p>")).toBe(
      "one\n\ntwo\nthree",
    );
    expect(htmlToPlainText("<div>one</div><div>two</div>")).toBe("one\ntwo");
  });

  it("collapses source indentation the way a browser renders it", () => {
    expect(htmlToPlainText("<div>\n  hello\n  world\n</div>")).toBe(
      "hello world",
    );
  });

  it("keeps whitespace inside <pre>", () => {
    expect(htmlToPlainText("<pre>if (x) {\n    go();\n}</pre>")).toBe(
      "if (x) {\n    go();\n}",
    );
  });

  it("ignores script and style content", () => {
    expect(
      htmlToPlainText("<style>p{color:red}</style><p>visible</p><script>x()</script>"),
    ).toBe("visible");
  });

  it("collapses the empty paragraphs Word and Docs pad with", () => {
    expect(htmlToPlainText("<p>one</p><p></p><p></p><p></p><p>two</p>")).toBe(
      "one\n\ntwo",
    );
  });

  it("flattens a list to one item per line", () => {
    expect(htmlToPlainText("<ul><li>first</li><li>second</li></ul>")).toBe(
      "first\nsecond",
    );
  });
});

describe("getClipboardText", () => {
  it("prefers text/plain", () => {
    const text = getClipboardText(
      clipboard({ "text/plain": "the text", "text/html": "<p>the markup</p>" }),
    );
    expect(text).toBe("the text");
  });

  it("falls back to text/html when no plain text is offered", () => {
    // A clipboard with markup but no text/plain flavour pastes as nothing
    // under Lexical's plain-text handler.
    expect(getClipboardText(clipboard({ "text/html": "<p>Hi <b>there</b></p>" }))).toBe(
      "Hi there",
    );
  });

  it("falls back to a URL list, dropping its comment lines", () => {
    expect(
      getClipboardText(
        clipboard({ "text/uri-list": "# comment\r\nhttps://example.com/a\r\n" }),
      ),
    ).toBe("https://example.com/a");
  });

  it("normalizes whatever flavour it picks", () => {
    expect(getClipboardText(clipboard({ "text/plain": "a\r\nb\u00a0c" }))).toBe(
      "a\nb c",
    );
  });

  it("returns empty for an empty clipboard", () => {
    expect(getClipboardText(clipboard({}))).toBe("");
  });

  it("survives a getData that throws", () => {
    const throwing: ClipboardPayload = {
      getData: () => {
        throw new Error("not available");
      },
    };
    expect(getClipboardText(throwing)).toBe("");
  });
});

describe("resolvePaste", () => {
  const png = new File(["fake"], "screenshot.png", { type: "image/png" });

  it("inserts text", () => {
    expect(resolvePaste(clipboard({ "text/plain": "hello" }), { canAttach: true })).toEqual({
      kind: "text",
      text: "hello",
    });
  });

  it("attaches a file when the clipboard carries no text", () => {
    const action = resolvePaste(clipboard({}, [png]), { canAttach: true });
    expect(action).toEqual({ kind: "files", files: [png] });
  });

  it("prefers text when the clipboard carries both", () => {
    // Copying spreadsheet cells or a captioned figure puts an image on the
    // clipboard alongside the text; attaching a screenshot instead of pasting
    // is the worse surprise.
    const action = resolvePaste(
      clipboard({ "text/plain": "Q3\t14000" }, [png]),
      { canAttach: true },
    );
    expect(action).toEqual({ kind: "text", text: "Q3\t14000" });
  });

  it("defers to the default handler when files can't be attached", () => {
    expect(resolvePaste(clipboard({}, [png]), { canAttach: false })).toEqual({
      kind: "default",
    });
  });

  it("defers to the default handler for an empty clipboard", () => {
    expect(resolvePaste(clipboard({}), { canAttach: true })).toEqual({
      kind: "default",
    });
  });
});
