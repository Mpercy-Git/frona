/**
 * Clipboard → plain text for the chat composer.
 *
 * The composer runs Lexical's `PlainTextPlugin`, whose paste handler reads only
 * `text/plain` (falling back to `text/uri-list`) and only tokenizes newlines
 * and tabs. Everything else on a real clipboard falls through it:
 *
 *   - an HTML-only payload (some rich editors and mail clients put no
 *     `text/plain` flavour on the clipboard) pastes as nothing at all;
 *   - CR-only line endings, U+2028/U+2029 separators and NEL — common in text
 *     copied out of PDFs, spreadsheets and older Mac apps — are inserted as
 *     literal control characters, so the lines visibly run together;
 *   - non-breaking spaces and zero-width characters from Word/Google Docs
 *     survive into the message, where they read as ordinary spaces but break
 *     the `/command` and `@agent` trigger match (which splits on real
 *     whitespace) and travel on to the agent as invisible junk.
 *
 * These helpers are pure so the behaviour can be tested without a DOM editor;
 * `ComposerPastePlugin` wires them to Lexical.
 */

/** CRLF, CR, NEL, LINE SEPARATOR, PARAGRAPH SEPARATOR → a plain newline. */
const LINE_SEPARATORS = /\r\n|[\r\u0085\u2028\u2029]/g;

/** Spaces that aren't U+0020: NBSP, the en/em quad family, narrow + ideographic. */
const SPACE_LOOKALIKES = /[\u00a0\u1680\u2000-\u200a\u202f\u205f\u3000]/g;

/** Zero-width and soft-hyphen characters: invisible, and they split words. */
const INVISIBLES = /[\u00ad\u200b-\u200d\u2060\ufeff]/g;

/** Control characters with no meaning in a message. Tabs and newlines are kept. */
const CONTROL_CHARS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g;

/** Elements whose contents are markup, not text. */
const NON_TEXT_TAGS = new Set(["script", "style", "noscript", "head", "title"]);

/** Elements that start and end a line when flattened to text. */
const LINE_TAGS = new Set([
  "dd", "div", "dt", "figcaption", "li", "tbody", "tfoot", "thead", "tr",
]);

/** Elements that read as their own paragraph, i.e. with a blank line around. */
const PARAGRAPH_TAGS = new Set([
  "address", "article", "aside", "blockquote", "dl", "fieldset", "figure",
  "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr",
  "main", "nav", "ol", "p", "pre", "section", "table", "ul",
]);

/** Elements whose whitespace is significant. */
const PREFORMATTED_TAGS = new Set(["pre", "textarea"]);

const TEXT_NODE = 3;
const ELEMENT_NODE = 1;

/**
 * Make pasted text safe to insert: one newline convention, real spaces, and no
 * invisible characters. Deliberately leaves the visible content alone — smart
 * quotes, em dashes and emoji are the user's text, not formatting noise.
 */
export function normalizePastedText(text: string): string {
  return text
    .replace(LINE_SEPARATORS, "\n")
    .replace(SPACE_LOOKALIKES, " ")
    .replace(INVISIBLES, "")
    .replace(CONTROL_CHARS, "");
}

/**
 * Break markers, emitted while walking the tree and turned into newlines at
 * the end. Text nodes are normalized on the way in — which strips control
 * characters — so a marker can never collide with pasted content.
 */
const LINE_BREAK = "\u0001";
const PARAGRAPH_BREAK = "\u0002";

/** A marker plus any whitespace and further markers touching it. */
const BREAK_RUN = /[ \t\n\u0001\u0002]*[\u0001\u0002][ \t\n\u0001\u0002]*/g;

/**
 * One newline per run of breaks — two if any of them was a paragraph. Runs
 * merge, so the empty paragraphs Word and Google Docs pad their markup with
 * cost nothing.
 */
function resolveBreaks(text: string): string {
  return text.replace(BREAK_RUN, (run) =>
    run.includes(PARAGRAPH_BREAK) ? "\n\n" : "\n",
  );
}

function walkHtml(node: Node, out: string[], preformatted: boolean): void {
  if (node.nodeType === TEXT_NODE) {
    const text = normalizePastedText(node.nodeValue ?? "");
    // Outside <pre>, collapse runs of whitespace the way a browser renders
    // them — newlines in the HTML source are indentation, not line breaks.
    out.push(preformatted ? text : text.replace(/\s+/g, " "));
    return;
  }
  if (node.nodeType !== ELEMENT_NODE) return;

  const element = node as Element;
  const tag = element.tagName.toLowerCase();
  if (NON_TEXT_TAGS.has(tag)) return;
  if (tag === "br") {
    out.push(LINE_BREAK);
    return;
  }

  const boundary = PARAGRAPH_TAGS.has(tag)
    ? PARAGRAPH_BREAK
    : LINE_TAGS.has(tag)
      ? LINE_BREAK
      : "";

  if (boundary) out.push(boundary);
  const inPre = preformatted || PREFORMATTED_TAGS.has(tag);
  for (const child of Array.from(element.childNodes)) {
    walkHtml(child, out, inPre);
  }
  if (boundary) out.push(boundary);
}

/**
 * Flatten a `text/html` clipboard payload to plain text. Parsed detached via
 * `DOMParser`, so nothing in the pasted markup is ever loaded or executed.
 */
export function htmlToPlainText(html: string): string {
  if (typeof DOMParser === "undefined") return "";
  const body = new DOMParser().parseFromString(html, "text/html").body;
  if (!body) return "";

  const out: string[] = [];
  for (const child of Array.from(body.childNodes)) walkHtml(child, out, false);
  return resolveBreaks(out.join("")).replace(/[ \t]+\n/g, "\n").trim();
}

/** `text/uri-list` is one URL per line, with `#` comment lines. */
function uriListToText(uriList: string): string {
  return uriList
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"))
    .join("\n");
}

/** `getData` throws in some browsers when the type isn't on the clipboard. */
function readType(data: Pick<DataTransfer, "getData">, type: string): string {
  try {
    return data.getData(type) ?? "";
  } catch {
    return "";
  }
}

/**
 * Best available text for a paste, in descending fidelity: what the source
 * says the text is, then a URL list, then the HTML flattened.
 */
export function getClipboardText(data: Pick<DataTransfer, "getData">): string {
  const plain = readType(data, "text/plain");
  if (plain.trim()) return normalizePastedText(plain);

  const uriList = readType(data, "text/uri-list");
  if (uriList.trim()) return normalizePastedText(uriListToText(uriList));

  const html = readType(data, "text/html");
  if (html.trim()) return htmlToPlainText(html);

  return "";
}

export type PasteAction =
  | { kind: "text"; text: string }
  | { kind: "files"; files: readonly File[] }
  | { kind: "default" };

export type ClipboardPayload = Pick<DataTransfer, "getData"> & {
  files?: FileList | null;
};

/**
 * Decide what a paste should do.
 *
 * Text wins over files when both are present: copying a range of spreadsheet
 * cells, or a figure with its caption, puts an image on the clipboard
 * *alongside* the text, and silently attaching a screenshot when the user meant
 * to paste the text is the more annoying failure. A screenshot or a copied file
 * carries no text, so it still attaches.
 */
export function resolvePaste(
  data: ClipboardPayload,
  options: { canAttach: boolean },
): PasteAction {
  const text = getClipboardText(data);
  if (text) return { kind: "text", text };

  const files = data.files ? Array.from(data.files) : [];
  if (files.length > 0 && options.canAttach) return { kind: "files", files };

  return { kind: "default" };
}
