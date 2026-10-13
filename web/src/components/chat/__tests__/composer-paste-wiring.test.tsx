import { useEffect } from "react";
import { describe, it, expect, vi } from "vitest";
import { render, act } from "@testing-library/react";
import { LexicalComposer } from "@lexical/react/LexicalComposer";
import { PlainTextPlugin } from "@lexical/react/LexicalPlainTextPlugin";
import { ContentEditable } from "@lexical/react/LexicalContentEditable";
import { LexicalErrorBoundary } from "@lexical/react/LexicalErrorBoundary";
import { useLexicalComposerContext } from "@lexical/react/LexicalComposerContext";
import { $getRoot, PASTE_COMMAND, type LexicalEditor } from "lexical";

vi.mock("@assistant-ui/react", () => ({
  useAui: () => ({ thread: { getState: () => ({ capabilities: {} }) }, composer: {} }),
}));

import { ComposerPastePlugin } from "../composer-paste-plugin";

// jsdom lacks the range geometry Lexical reads when it scrolls the caret into view.
Range.prototype.getBoundingClientRect = () => new DOMRect();
Range.prototype.getClientRects = () => ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList;

const grabbed: { editor?: LexicalEditor } = {};
function Grab() {
  const [editor] = useLexicalComposerContext();
  useEffect(() => {
    grabbed.editor = editor;
  }, [editor]);
  return null;
}

function setup() {
  const view = render(
    <LexicalComposer initialConfig={{ namespace: "t", onError: (e) => { throw e; } }}>
      <PlainTextPlugin
        contentEditable={<ContentEditable data-testid="ce" />}
        placeholder={null}
        ErrorBoundary={LexicalErrorBoundary}
      />
      <ComposerPastePlugin />
      <Grab />
    </LexicalComposer>,
  );
  return view.getByTestId("ce");
}

describe("ComposerPastePlugin wiring", () => {
  it.each(["insertFromPaste", "insertFromPasteAsQuotation"])(
    "inserts text carried directly by %s without clipboardData",
    async (inputType) => {
      const ce = setup();
      ce.focus();
      const event = new InputEvent("beforeinput", {
        inputType,
        data: "pasted text",
        bubbles: true,
        cancelable: true,
      });
      await act(async () => { ce.dispatchEvent(event); });
      expect(event.defaultPrevented).toBe(true);
      expect(grabbed.editor!.getEditorState().read(() => $getRoot().getTextContent())).toBe("pasted text");
    },
  );

  it("handles an InputEvent dispatched through PASTE_COMMAND", async () => {
    setup().focus();
    const event = new InputEvent("beforeinput", {
      inputType: "insertFromPaste",
      data: "direct paste",
      cancelable: true,
    });
    await act(async () => { grabbed.editor!.dispatchCommand(PASTE_COMMAND, event); });
    expect(grabbed.editor!.getEditorState().read(() => $getRoot().getTextContent())).toBe("direct paste");
  });

  it.each(["insertText", "insertReplacementText", "insertFromPaste"])(
    "inserts once for a paste followed by a single-line %s",
    async (inputType) => {
      const ce = setup();
      ce.focus();
      const paste = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(paste, "clipboardData", {
        value: { getData: (type: string) => type === "text/plain" ? "hello world" : "" },
      });
      const input = new InputEvent("beforeinput", {
        inputType,
        data: "hello world",
        bubbles: true,
        cancelable: true,
      });
      await act(async () => {
        ce.dispatchEvent(paste);
        ce.dispatchEvent(input);
      });
      expect(input.defaultPrevented).toBe(true);
      expect(grabbed.editor!.getEditorState().read(() => $getRoot().getTextContent())).toBe("hello world");
    },
  );

  it("does not intercept a sibling input's multi-line event", async () => {
    const ce = setup();
    const sibling = document.createElement("textarea");
    ce.parentElement!.append(sibling);
    const input = new InputEvent("beforeinput", {
      inputType: "insertText",
      data: "one\ntwo",
      bubbles: true,
      cancelable: true,
    });
    await act(async () => { sibling.dispatchEvent(input); });
    expect(input.defaultPrevented).toBe(false);
    expect(grabbed.editor!.getEditorState().read(() => $getRoot().getTextContent())).toBe("");
  });

  it.each(["insertText", "insertReplacementText"])("leaves ordinary single-line %s to Lexical", async (inputType) => {
    const ce = setup();
    ce.focus();
    const input = new InputEvent("beforeinput", {
      inputType, data: "ordinary input", bubbles: true, cancelable: true,
    });
    await act(async () => { ce.dispatchEvent(input); });
    expect(input.defaultPrevented).toBe(false);
  });

  it("allows the same paste after an intervening edit", async () => {
    const ce = setup();
    ce.focus();
    const paste = () => new InputEvent("beforeinput", {
      inputType: "insertFromPaste", data: "hello", bubbles: true, cancelable: true,
    });
    const secondPaste = paste();
    await act(async () => {
      ce.dispatchEvent(paste());
      ce.dispatchEvent(new InputEvent("beforeinput", {
        inputType: "deleteContentBackward", bubbles: true, cancelable: true,
      }));
      grabbed.editor!.update(() => { $getRoot().clear(); }, { discrete: true });
      ce.dispatchEvent(secondPaste);
    });
    expect(secondPaste.defaultPrevented).toBe(true);
    expect(grabbed.editor!.getEditorState().read(() => $getRoot().getTextContent())).toBe("hello");
  });

  it("inserts a multi-line beforeinput insertText as a paste", async () => {
    const ce = setup();
    ce.focus();
    const ev = new InputEvent("beforeinput", {
      inputType: "insertText",
      data: "one\ntwo\nthree",
      bubbles: true,
      cancelable: true,
    });
    await act(async () => {
      ce.dispatchEvent(ev);
      await new Promise((r) => setTimeout(r, 300));
    });
    expect(ev.defaultPrevented).toBe(true);
    expect(grabbed.editor!.getEditorState().read(() => $getRoot().getTextContent())).toBe("one\ntwo\nthree");
  });

  it("inserts once when the same paste is reported twice", async () => {
    const ce = setup();
    ce.focus();
    const fire = () =>
      ce.dispatchEvent(
        new InputEvent("beforeinput", {
          inputType: "insertText",
          data: "one\ntwo",
          bubbles: true,
          cancelable: true,
        }),
      );
    await act(async () => {
      fire();
      fire();
      await new Promise((r) => setTimeout(r, 300));
    });
    expect(grabbed.editor!.getEditorState().read(() => $getRoot().getTextContent())).toBe("one\ntwo");
  });
});
