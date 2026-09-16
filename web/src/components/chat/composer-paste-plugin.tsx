"use client";

import { useEffect } from "react";
import { useLexicalComposerContext } from "@lexical/react/LexicalComposerContext";
import {
  $generateNodesFromRawText,
  $getRoot,
  $getSelection,
  $isNodeSelection,
  $isRangeSelection,
  COMMAND_PRIORITY_CRITICAL,
  PASTE_COMMAND,
  PASTE_TAG,
} from "lexical";
import { useAui } from "@assistant-ui/react";

import { resolvePaste, type ClipboardPayload } from "@/lib/paste-text";

/** PASTE_COMMAND carries a ClipboardEvent, an InputEvent or a KeyboardEvent. */
function getPayload(event: unknown): ClipboardPayload | null {
  if (!event || typeof event !== "object") return null;
  const data =
    (event as ClipboardEvent).clipboardData ??
    (event as InputEvent).dataTransfer ??
    null;
  return data && typeof data.getData === "function" ? data : null;
}

/** Exported for tests; call inside an `editor.update`. */
export function $insertPastedText(text: string): void {
  const selection = $getSelection();

  if ($isRangeSelection(selection)) {
    selection.insertRawText(text);
    return;
  }

  if ($isNodeSelection(selection)) {
    // `NodeSelection.insertRawText` is a no-op in Lexical, so with a directive
    // chip selected (one click on `@agent` does it) the paste vanishes.
    // `insertNodes` does replace the selection, so route through that.
    selection.insertNodes($generateNodesFromRawText(text));
    return;
  }

  // No selection: the editor holds focus but has never had a caret placed in
  // it. Append rather than drop the paste.
  const end = $getRoot().selectEnd();
  end.insertRawText(text);
}

/**
 * Paste handling for the Lexical composer.
 *
 * `PlainTextPlugin` only looks at `text/plain` and only inserts into a range
 * selection, which makes a paste silently do nothing for HTML-only clipboards,
 * for a selected directive chip, and for any file or screenshot — and lets
 * stray control characters through when it does insert. This runs ahead of it
 * (`COMMAND_PRIORITY_CRITICAL`) and falls back to the default handler when it
 * has nothing better to offer.
 *
 * Rendered as a child of `LexicalComposerInput`, i.e. inside the composer's
 * Lexical context.
 */
export function ComposerPastePlugin() {
  const [editor] = useLexicalComposerContext();
  const aui = useAui();

  useEffect(() => {
    return editor.registerCommand(
      PASTE_COMMAND,
      (event) => {
        const payload = getPayload(event);
        if (!payload) return false;

        const canAttach = !!aui.thread.getState().capabilities.attachments;
        const action = resolvePaste(payload, { canAttach });
        if (action.kind === "default") return false;

        event?.preventDefault();

        if (action.kind === "files") {
          // The attachment adapter surfaces its own upload failures as toasts.
          for (const file of action.files) {
            void aui.composer.addAttachment(file).catch(() => {});
          }
          return true;
        }

        editor.update(
          () => {
            $insertPastedText(action.text);
          },
          // Same tag Lexical's own paste uses: it makes the paste its own
          // undo entry instead of merging with whatever was typed before it.
          { tag: PASTE_TAG },
        );
        return true;
      },
      COMMAND_PRIORITY_CRITICAL,
    );
  }, [editor, aui]);

  return null;
}
