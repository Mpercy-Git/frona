"use client";

import { DocumentTextIcon } from "@heroicons/react/24/outline";
import { strListArg } from "./args";
import { ToolViewFallback } from "./safe-tool-view";
import { ToolRow } from "./tool-row";
import type { ToolView } from "./types";

interface MemoryHit {
  name: string;
  tag: string;
  description: string;
  path: string;
  /** The page's own prose, which the result now carries inline. */
  page: string | null;
}

/**
 * Parse the `memory_search` result text emitted by the backend
 * (crates/frona-server/src/memory/pkm/tools.rs). Keep in lockstep:
 *
 *   Top matches, page text included — call read(paths=[…]) only for …
 *
 *   - Name  [tag]
 *     description
 *     path/to/page
 *   <page path="path/to/page">
 *   the page's prose
 *   </page>
 *
 * A hit's first three lines are fixed; everything after them is the inline
 * page block, or a one-line note in its place (frontmatter only, text already
 * sent, budget spent). Reading the path off the LAST line of the block - which
 * is what this did before the text was inlined - now picks up `</page>` and
 * folds the whole page into the description.
 *
 * Returns [] for the empty ("No pages matched") case and null on parse
 * failure so the caller can fall back to raw text.
 */
function parseMemoryResult(text: string): MemoryHit[] | null {
  const trimmed = text.trim();
  if (!trimmed) return [];
  if (trimmed.startsWith("No pages matched")) return [];

  const hits: MemoryHit[] = [];
  for (const block of text.split(/\n\n+/)) {
    const lines = block.split("\n").filter((l) => l.trim().length > 0);
    if (lines.length === 0) continue;
    // The header line ("Top matches - …") and any stray prose don't start an item.
    if (!lines[0].startsWith("- ")) continue;

    // The tag is empty for a page the Classify stage has not typed yet - the backend
    // emits `- Name  []`. Requiring a non-empty tag here failed the whole parse, so a
    // single untyped hit dropped the entire result back to raw text.
    const head = lines[0].match(/^-\s+(.*?)\s+\[(.*?)\]\s*$/);
    if (!head || lines.length < 3) return null;
    const rest = lines.slice(3);
    const openIdx = rest.findIndex((l) => l.trimStart().startsWith("<page"));
    const closeIdx = rest.findIndex((l) => l.trim() === "</page>");
    const page =
      openIdx !== -1 && closeIdx > openIdx
        ? rest
            .slice(openIdx + 1, closeIdx)
            .join("\n")
            .trim()
        : null;
    hits.push({
      name: head[1].trim(),
      tag: head[2].trim(),
      description: lines[1].trim(),
      path: lines[2].trim(),
      page: page || null,
    });
  }

  // Non-empty text that yielded no items is a format mismatch, not "no results".
  return hits.length > 0 ? hits : null;
}

export const MemorySearchView: ToolView = ({
  args,
  result,
  status,
  isExpanded,
  onToggle,
}) => {
  // A batched search carries several queries; each answers under its own heading in
  // the result, which the parser below skips over, so the hits still list as one set.
  const query = strListArg(args, "query", "queries").join(" · ");
  const resultText =
    typeof result === "string"
      ? result
      : result !== undefined
        ? JSON.stringify(result, null, 2)
        : "";
  const hits = parseMemoryResult(resultText);
  if (result !== undefined && hits === null) {
    return <ToolViewFallback />;
  }

  const subtitle =
    hits && hits.length > 0
      ? `${query ? `${query} · ` : ""}${hits.length} ${hits.length === 1 ? "match" : "matches"}`
      : query || null;

  return (
    <ToolRow status={status} expandable={resultText.length > 0}>
      <ToolRow.Header onToggle={onToggle} isExpanded={isExpanded}>
        <ToolRow.Title>Memory Search</ToolRow.Title>
        <ToolRow.Subtitle>{subtitle}</ToolRow.Subtitle>
      </ToolRow.Header>

      <ToolRow.Body isExpanded={isExpanded} unstyled>
        <div className="p-3">
          {!hits || hits.length === 0 ? (
            <p className="text-xs text-text-tertiary m-0">No pages matched.</p>
          ) : (
            <ol className="flex flex-col gap-3 m-0 p-0 list-none">
              {hits.map((hit, i) => (
                <li key={`${i}-${hit.path}`} className="flex flex-col gap-0.5">
                  <div className="flex items-center gap-1.5">
                    <span className="text-sm font-medium text-text-primary break-words">
                      {hit.name}
                    </span>
                    {hit.tag && (
                      <span className="inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium bg-surface-tertiary text-text-tertiary">
                        {hit.tag}
                      </span>
                    )}
                  </div>
                  {hit.description && (
                    <p className="text-xs text-text-secondary m-0">{hit.description}</p>
                  )}
                  {hit.path && (
                    <span className="inline-flex items-center gap-1 text-xs font-mono text-text-tertiary break-all">
                      <DocumentTextIcon className="h-3 w-3 shrink-0" />
                      {hit.path}
                    </span>
                  )}
                  {hit.page && (
                    <p className="text-xs text-text-tertiary m-0 mt-1 whitespace-pre-wrap border-l-2 border-border-secondary pl-2">
                      {hit.page}
                    </p>
                  )}
                </li>
              ))}
            </ol>
          )}
        </div>
      </ToolRow.Body>
    </ToolRow>
  );
};
