"use client";

import { ArrowTopRightOnSquareIcon } from "@heroicons/react/24/outline";
import { toString as mdastToString } from "mdast-util-to-string";
import ReactMarkdown from "react-markdown";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import { unified } from "unified";

export interface ExternalLink {
  label: string;
  url: string;
}

/**
 * Extract plain text from a markdown source. Built on the same `unified` /
 * `remark-parse` pipeline `react-markdown` uses internally, so it handles
 * escape sequences, code blocks, tables, and nested formatting the same way
 * the renderer does - no regex parsing.
 *
 * Intended for layouts that can't host a renderer (line-clamped previews,
 * tooltip text). For anything visible to the user as prose, use
 * `<ManifestInfo />` instead.
 */
export function markdownToPlainText(source: string): string {
  const tree = unified().use(remarkParse).parse(source);
  return mdastToString(tree).replace(/\s+/g, " ").trim();
}

export interface ManifestInfoData {
  description: string;
  setup_instructions?: string | null;
  external_links?: ExternalLink[];
}

const PROSE_CLASSES =
  "prose prose-sm max-w-none text-[var(--text-primary)] " +
  "prose-headings:text-[var(--text-primary)] prose-strong:text-[var(--text-primary)] " +
  "prose-a:text-[var(--accent)] prose-a:no-underline hover:prose-a:underline " +
  "prose-code:text-[var(--text-primary)] prose-code:before:content-none prose-code:after:content-none " +
  "prose-blockquote:text-[var(--text-secondary)] prose-blockquote:border-[var(--border)] " +
  "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0";

/**
 * Render a markdown string with the shared channel prose styling. Exported so
 * callers (e.g. a standalone "Setup" panel) can reuse the same look without
 * duplicating the className soup.
 */
export function MarkdownProse({ source }: { source: string }) {
  return (
    <div className={PROSE_CLASSES}>
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{source}</ReactMarkdown>
    </div>
  );
}

/**
 * Renders a channel manifest's user-facing prose: the markdown description
 * and the external policy/docs links. Setup instructions are intentionally
 * NOT rendered here - callers that want them should drop a separate panel
 * with `<MarkdownProse source={manifest.setup_instructions} />`.
 *
 * Generic - used both in the channel-create dialog and the channel detail page.
 */
export function ManifestInfo({ manifest }: { manifest: ManifestInfoData }) {
  const links = manifest.external_links ?? [];

  return (
    <div className="space-y-4">
      <MarkdownProse source={manifest.description} />

      {links.length > 0 && (
        <ul className="space-y-1.5">
          {links.map((link) => (
            <li key={link.url}>
              <a
                href={link.url}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 text-xs text-accent hover:underline"
              >
                <ArrowTopRightOnSquareIcon className="h-3.5 w-3.5 shrink-0" />
                {link.label}
              </a>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
