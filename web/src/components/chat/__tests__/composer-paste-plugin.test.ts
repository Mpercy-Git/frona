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

import { $insertPastedText } from "../composer-paste-plugin";

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
