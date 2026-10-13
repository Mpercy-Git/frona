"use client";

import { useEffect, useRef } from "react";
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
  getClipboardText,
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

/**
 * Android keyboards can report one paste through more than one route (a
 * `paste` event and a `beforeinput`), and each route inserts on its own. The
 * same text arriving twice inside this window is one paste, not two.
 */
const DUPLICATE_PASTE_WINDOW_MS = 500;

/** Returns a guard that is true for a repeat of the previous paste. */
export function createPasteDeduper(now: () => number = () => performance.now()) {
  let last: { text: string; at: number } | null = null;
  const matches = (text: string) =>
    !!last && last.text === text && now() - last.at < DUPLICATE_PASTE_WINDOW_MS;
  const isDuplicate = (text: string): boolean => {
    const at = now();
    const repeat = !!last && last.text === text && at - last.at < DUPLICATE_PASTE_WINDOW_MS;
    // Repeated notifications must not keep extending the suppression window.
    if (!repeat) last = { text, at };
    return repeat;
  };
  return Object.assign(isDuplicate, { matches, reset: () => { last = null; } });
}

/** Any line break flavour; `normalizePastedText` maps them all to `\n`. */
const MULTILINE = /[\r\n\u0085\u2028\u2029]/;

function getInputText(event: InputEvent): string {
  return event.data || (event.dataTransfer ? getClipboardText(event.dataTransfer) : "");
}

function isPasteInput(event: InputEvent): boolean {
  return event.inputType === "insertFromPaste" || event.inputType === "insertFromPasteAsQuotation";
}

function isTextInput(event: InputEvent): boolean {
  return event.inputType === "insertText" || event.inputType === "insertReplacementText";
}

/**
 * Text carried by an explicit paste input, or a multiline keyboard insertion.
 *
 * Android keyboards (Gboard's clipboard chip and its long-press Paste) often
 * skip the `paste` event and deliver the whole clipboard as an `insertText`
 * whose `data` holds every line. Lexical inserts that string into a single
 * text node, so the line breaks never become paragraphs and the message
 * arrives mangled. Explicit insertFromPaste inputs can carry a single line
 * directly in `data`, too. Ordinary single-line input (typing, autocorrect,
 * dictation) is left to Lexical.
 */
export function getBeforeInputPasteText(event: InputEvent): string | null {
  if (!isPasteInput(event) && !isTextInput(event)) {
    return null;
  }
  const text = getInputText(event);
  return text && (isPasteInput(event) || MULTILINE.test(text)) ? normalizePastedText(text) : null;
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
  const isDuplicatePaste = useRef(createPasteDeduper()).current;

  useEffect(() => {
    return editor.registerCommand(
      PASTE_COMMAND,
      (event) => {
        const payload = getPayload(event);
        const canAttach = !!aui.thread.getState().capabilities.attachments;
        let action = payload ? resolvePaste(payload, { canAttach }) : { kind: "default" as const };
        // Some mobile paste InputEvents carry only `data`, with no clipboard
        // object. Lexical's plain-text fallback prevents default but inserts
        // nothing for those events.
        if (action.kind === "default" && event) {
          const text = getBeforeInputPasteText(event as InputEvent);
          if (text) action = { kind: "text", text };
        }
        if (action.kind === "default") return false;

        event?.preventDefault();

        if (action.kind === "files") {
          // The attachment adapter surfaces its own upload failures as toasts.
          for (const file of action.files) {
            void aui.composer.addAttachment(file).catch(() => {});
          }
          return true;
        }

        if (isDuplicatePaste(action.text)) return true;
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
  }, [editor, aui, isDuplicatePaste]);

  // Capture phase on the root's parent, so it runs before Lexical's own
  // `beforeinput` listener on the root itself.
  useEffect(() => {
    return editor.registerRootListener((root, previous) => {
      previous?.parentElement?.removeEventListener("beforeinput", onBeforeInput, true);
      root?.parentElement?.addEventListener("beforeinput", onBeforeInput, true);
    });

    function onBeforeInput(event: Event) {
      const root = editor.getRootElement();
      if (!root || !(event.target instanceof Node) || !root.contains(event.target)) return;
      const input = event as InputEvent;
      if (input.isComposing || !input.cancelable) {
        isDuplicatePaste.reset();
        return;
      }
      let text = getBeforeInputPasteText(input);
      // A single-line insertText is normally typing/autocorrect. Intercept it
      // only when it repeats text we have just inserted through a paste path.
      if (!text && isTextInput(input)) {
        const candidate = normalizePastedText(getInputText(input));
        if (candidate && isDuplicatePaste.matches(candidate)) text = candidate;
      }
      if (!text) {
        isDuplicatePaste.reset();
        return;
      }
      input.preventDefault();
      input.stopImmediatePropagation();
      if (isDuplicatePaste(text)) return;
      editor.update(() => $insertPastedText(text), { tag: PASTE_TAG });
    }
  }, [editor, isDuplicatePaste]);

  return null;
}
