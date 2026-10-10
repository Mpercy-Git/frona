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

import {
  normalizePastedText,
  resolvePaste,
  type ClipboardPayload,
} from "@/lib/paste-text";

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

/** Any line break flavour; `normalizePastedText` maps them all to `\n`. */
const MULTILINE = /[\r\n\u0085\u2028\u2029]/;

/**
 * Text carried by a `beforeinput` text insertion that is really a paste.
 *
 * Android keyboards (Gboard's clipboard chip and its long-press Paste) often
 * skip the `paste` event and deliver the whole clipboard as an `insertText`
 * whose `data` holds every line. Lexical inserts that string into a single
 * text node, so the line breaks never become paragraphs and the message
 * arrives mangled. Single-line input (typing, autocorrect, dictation) is left
 * to Lexical.
 */
export function getBeforeInputPasteText(event: InputEvent): string | null {
  if (event.inputType !== "insertText" && event.inputType !== "insertReplacementText") {
    return null;
  }
  let text = event.data ?? "";
  if (!text && event.dataTransfer) {
    try {
      text = event.dataTransfer.getData("text/plain");
    } catch {
      text = "";
    }
  }
  return MULTILINE.test(text) ? normalizePastedText(text) : null;
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

  // Capture phase on the root's parent, so it runs before Lexical's own
  // `beforeinput` listener on the root itself.
  useEffect(() => {
    return editor.registerRootListener((root, previous) => {
      previous?.parentElement?.removeEventListener("beforeinput", onBeforeInput, true);
      root?.parentElement?.addEventListener("beforeinput", onBeforeInput, true);
    });

    function onBeforeInput(event: Event) {
      const input = event as InputEvent;
      if (input.isComposing || !input.cancelable) return;
      const text = getBeforeInputPasteText(input);
      if (!text) return;
      input.preventDefault();
      input.stopImmediatePropagation();
      editor.update(() => $insertPastedText(text), { tag: PASTE_TAG });
    }
  }, [editor]);

  return null;
}
