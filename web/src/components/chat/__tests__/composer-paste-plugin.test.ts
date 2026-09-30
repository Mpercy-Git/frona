import { describe, it, expect } from "vitest";
import {
  $createNodeSelection,
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  $setSelection,
  createEditor,
  type LexicalEditor,
} from "lexical";
import { $createDirectiveNode, DirectiveNode } from "@assistant-ui/react-lexical";

import { $insertPastedText, getBeforeInputPasteText } from "../composer-paste-plugin";

function makeEditor(build: () => void): LexicalEditor {
  const editor = createEditor({
    nodes: [DirectiveNode],
    onError: (error) => {
      throw error;
    },
  });
  editor.update(build, { discrete: true });
  return editor;
}

function textOf(editor: LexicalEditor): string {
  return editor.getEditorState().read(() => $getRoot().getTextContent());
}

describe("$insertPastedText", () => {
  it("inserts at the caret", () => {
    const editor = makeEditor(() => {
      const paragraph = $createParagraphNode();
      paragraph.append($createTextNode("hello "));
      $getRoot().clear().append(paragraph);
      paragraph.selectEnd();
    });

    editor.update(() => $insertPastedText("world"), { discrete: true });

    expect(textOf(editor)).toBe("hello world");
  });

  it("turns newlines and tabs into real nodes", () => {
    const editor = makeEditor(() => {
      const paragraph = $createParagraphNode();
      $getRoot().clear().append(paragraph);
      paragraph.selectEnd();
    });

    editor.update(() => $insertPastedText("one\ttwo\nthree"), { discrete: true });

    expect(textOf(editor)).toBe("one\ttwo\nthree");
  });

  it("replaces a selected directive chip", () => {
    // One click on an `@agent` chip leaves a NodeSelection, and Lexical's
    // `NodeSelection.insertRawText` is a no-op — so the plain-text paste
    // handler silently swallowed the paste.
    const editor = makeEditor(() => {
      const paragraph = $createParagraphNode();
      const chip = $createDirectiveNode(
        { id: "agent:scout", type: "agent", label: "@Scout" },
        "@scout ",
      );
      paragraph.append(chip);
      $getRoot().clear().append(paragraph);

      const selection = $createNodeSelection();
      selection.add(chip.getKey());
      $setSelection(selection);
    });

    editor.update(() => $insertPastedText("pasted"), { discrete: true });

    expect(textOf(editor)).toBe("pasted");
  });

  it("appends when nothing is selected", () => {
    const editor = makeEditor(() => {
      const paragraph = $createParagraphNode();
      paragraph.append($createTextNode("draft "));
      $getRoot().clear().append(paragraph);
      $setSelection(null);
    });

    editor.update(() => $insertPastedText("tail"), { discrete: true });

    expect(textOf(editor)).toBe("draft tail");
  });
});

describe("getBeforeInputPasteText", () => {
  const input = (init: { data?: string | null; inputType?: string; dataTransfer?: unknown }) =>
    ({ inputType: "insertText", ...init }) as unknown as InputEvent;

  it("treats a multi-line insertText as a paste and normalises it", () => {
    expect(getBeforeInputPasteText(input({ data: "a\r\nb\u00a0c" }))).toBe("a\nb c");
  });

  it("reads the text from dataTransfer when data is null", () => {
    const dataTransfer = { getData: () => "one\ntwo" };
    expect(getBeforeInputPasteText(input({ data: null, dataTransfer }))).toBe("one\ntwo");
  });

  it("leaves single-line typing and other input types to Lexical", () => {
    expect(getBeforeInputPasteText(input({ data: "hello world" }))).toBeNull();
    expect(getBeforeInputPasteText(input({ data: "a\nb", inputType: "deleteContentBackward" }))).toBeNull();
  });
});
