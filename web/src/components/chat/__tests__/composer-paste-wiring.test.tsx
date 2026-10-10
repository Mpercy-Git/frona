import { describe, it, expect, vi } from "vitest";
import { render, act } from "@testing-library/react";
import { LexicalComposer } from "@lexical/react/LexicalComposer";
import { PlainTextPlugin } from "@lexical/react/LexicalPlainTextPlugin";
import { ContentEditable } from "@lexical/react/LexicalContentEditable";
import { LexicalErrorBoundary } from "@lexical/react/LexicalErrorBoundary";
import { useLexicalComposerContext } from "@lexical/react/LexicalComposerContext";
import { $getRoot, type LexicalEditor } from "lexical";

vi.mock("@assistant-ui/react", () => ({
  useAui: () => ({ thread: { getState: () => ({ capabilities: {} }) }, composer: {} }),
}));

import { ComposerPastePlugin } from "../composer-paste-plugin";

// jsdom lacks the range geometry Lexical reads when it scrolls the caret into view.
Range.prototype.getBoundingClientRect = () => new DOMRect();
Range.prototype.getClientRects = () => ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList;

let editorRef: LexicalEditor;
function Grab() {
  [editorRef] = useLexicalComposerContext();
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
    expect(editorRef.getEditorState().read(() => $getRoot().getTextContent())).toBe("one\ntwo\nthree");
  });
});
