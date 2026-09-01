"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowLeftIcon,
  ArrowTopRightOnSquareIcon,
  ChevronRightIcon,
  CircleStackIcon,
  MagnifyingGlassIcon,
  XMarkIcon,
} from "@heroicons/react/24/outline";
import { getMemoryPage } from "@/lib/api-client";
import { wikilinksToMarkdown } from "@/lib/memory-graph";
import type { AtomicMemory, MemoryPageResponse, MemorySearchResult, PageRelation } from "@/lib/memory-types";
import { CodeBlock } from "@/components/ui/code-block";

export type MemoryTab = "page" | "structure" | "memory";

interface MemoryInspectorProps {
  open: boolean;
  mobile: boolean;
  selectedPath: string;
  tab: MemoryTab;
  searchMode: boolean;
  searchQuery: string;
  searchResults: MemorySearchResult[];
  browsablePages: MemorySearchResult[];
  searchLoading: boolean;
  onClose: () => void;
  onSelect: (path: string, fromSearch?: boolean) => void;
  onTab: (tab: MemoryTab) => void;
  onBackToSearch: () => void;
}

function memoryCountLabel(count: number) {
  if (count === 0) return "No memories";
  return `${count} ${count === 1 ? "memory" : "memories"}`;
}

function valueText(value: unknown) {
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function relationList(
  title: string,
  relations: PageRelation[],
  selectedPath: string,
  onSelect: (path: string) => void,
) {
  return (
    <section>
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-text-tertiary">{title} ({relations.length})</h3>
      {relations.length === 0 ? (
        <p className="text-sm text-text-tertiary">None</p>
      ) : (
        <div className="space-y-2">
          {relations.map((relation) => {
            const connectedPath = relation.fromPath === selectedPath ? relation.toPath : relation.fromPath;
            return (
              <button
                key={relation.id}
                onClick={() => onSelect(connectedPath)}
                className="w-full rounded-lg border border-border bg-surface p-3 text-left hover:border-text-tertiary"
              >
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm font-medium text-text-primary">{relation.connectedName}</span>
                  <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-tertiary">{relation.origin}</span>
                  <ChevronRightIcon className="h-4 w-4 text-text-tertiary" />
                </div>
                <p className="mt-1 text-xs text-text-secondary">{relation.label}</p>
                <p className="mt-1 truncate font-mono text-[10px] text-text-tertiary">{relation.relation}</p>
                {relation.sourceMemoryIds.length > 0 && (
                  <p className="mt-1 text-[10px] text-text-tertiary">Memory: {relation.sourceMemoryIds.join(", ")}</p>
                )}
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}

function evidenceDestination(memory: AtomicMemory): string | null {
  for (const evidence of memory.evidence) {
    const source = evidence.source;
    const message = source.user_message ?? source.agent_message;
    if (message?.chat_id) return `/chat?id=${encodeURIComponent(String(message.chat_id))}`;
    if (source.task?.task_id) return `/chat?task=${encodeURIComponent(String(source.task.task_id))}`;
  }
  return null;
}

function SearchResults({
  query,
  results,
  browsablePages,
  loading,
  onSelect,
}: {
  query: string;
  results: MemorySearchResult[];
  browsablePages: MemorySearchResult[];
  loading: boolean;
  onSelect: (path: string) => void;
}) {
  const hasQuery = Boolean(query.trim());
  const displayedPages = hasQuery ? results : browsablePages;
  // A page with no atomic memories behind it is a skeleton consolidation has not filled
  // in yet. When every page is one, the panel would otherwise look like a working
  // browser over nothing, so say what is actually going on.
  const nothingLearned = !hasQuery
    && displayedPages.length > 0
    && displayedPages.every((page) => page.memoryCount === 0);

  return (
    <div className="h-full overflow-y-auto p-4 pr-14">
      <div className="mb-4 flex items-center gap-2">
        {hasQuery
          ? <MagnifyingGlassIcon className="h-5 w-5 text-text-tertiary" />
          : <CircleStackIcon className="h-5 w-5 text-text-tertiary" />}
        <h2 className="text-lg font-semibold text-text-primary">{hasQuery ? "Search memory" : "Browse memory"}</h2>
      </div>
      {hasQuery && loading ? (
        <p className="text-sm text-text-secondary">Searching…</p>
      ) : hasQuery && displayedPages.length === 0 ? (
        <p className="text-sm text-text-secondary">No memory pages match “{query}”.</p>
      ) : (
        <div className="space-y-2">
          <p className="mb-3 text-xs text-text-tertiary">
            {displayedPages.length} {hasQuery ? `result${displayedPages.length === 1 ? "" : "s"}` : `page${displayedPages.length === 1 ? "" : "s"}`}
          </p>
          {nothingLearned && (
            <div className="mb-3 rounded-xl border border-border bg-surface p-3">
              <p className="text-sm font-medium text-text-primary">Nothing has been learned yet</p>
              <p className="mt-1 text-sm leading-5 text-text-secondary">
                These pages are still empty outlines. Frona fills them in during consolidation, which runs in the
                background once a conversation has been idle for a while — check the consolidation status for
                whether a run has happened or failed.
              </p>
            </div>
          )}
          {displayedPages.map((result) => (
            <button
              key={result.path}
              onClick={() => onSelect(result.path)}
              className="w-full rounded-xl border border-border bg-surface p-3 text-left transition hover:border-text-tertiary hover:shadow-sm"
            >
              <div className="flex items-start justify-between gap-3">
                <span className="font-medium text-text-primary">{result.name}</span>
                <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-tertiary">{result.category}</span>
              </div>
              <p className="mt-1 line-clamp-2 text-sm leading-5 text-text-secondary">{result.description}</p>
              <div className="mt-2 flex items-baseline justify-between gap-3">
                <p className="min-w-0 truncate font-mono text-[11px] text-text-tertiary">{result.path}</p>
                {result.memoryCount !== undefined && (
                  <span className="shrink-0 text-[11px] text-text-tertiary">{memoryCountLabel(result.memoryCount)}</span>
                )}
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function PageBody({ data, onSelect }: { data: MemoryPageResponse; onSelect: (path: string) => void }) {
  // Skeleton pages (the seeded self-entity, or anything consolidation has created but not
  // yet authored) carry an empty body, which rendered as a blank panel.
  if (!data.page.body.trim()) {
    return (
      <div className="rounded-xl border border-border bg-surface p-4">
        <p className="text-sm font-medium text-text-primary">No page written yet</p>
        <p className="mt-1 text-sm leading-5 text-text-secondary">
          {data.memories.length === 0
            ? `Nothing has been learned about ${data.page.name} yet, so there is nothing to summarise. Frona writes this page during consolidation, once it has memories to draw on.`
            : `Frona has ${memoryCountLabel(data.memories.length).toLowerCase()} for ${data.page.name} but has not written the summary yet. The Memory tab shows the underlying memories.`}
        </p>
      </div>
    );
  }

  return (
    <article className="prose prose-sm max-w-none text-text-primary prose-headings:text-text-primary prose-p:text-text-secondary prose-li:text-text-secondary prose-strong:text-text-primary prose-a:text-accent prose-code:text-text-primary prose-code:before:content-none prose-code:after:content-none prose-pre:bg-transparent prose-pre:p-0 prose-blockquote:text-text-secondary prose-blockquote:border-border prose-hr:border-border prose-th:border-border prose-td:border-border">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={(url) => url.startsWith("memory:") ? url : defaultUrlTransform(url)}
        components={{
          a: ({ href, children }) => href?.startsWith("memory:") ? (
            <button
              onClick={() => onSelect(decodeURIComponent(href.slice("memory:".length)))}
              className="cursor-pointer font-medium text-accent underline decoration-accent/40 underline-offset-2"
            >
              {children}
            </button>
          ) : (
            <a href={href} target="_blank" rel="noreferrer" className="cursor-pointer">{children}</a>
          ),
          code: ({ className, children, ...props }) => {
            const language = /language-(\w+)/.exec(className || "")?.[1];
            const code = String(children).replace(/\n$/, "");
            if (language || String(children).includes("\n")) {
              return <CodeBlock code={code} language={language || "text"} />;
            }
            return (
              <code className={`rounded bg-surface-tertiary px-1 py-0.5 font-mono text-[0.85em] text-text-primary ${className || ""}`} {...props}>
                {children}
              </code>
            );
          },
        }}
      >
        {wikilinksToMarkdown(data.page.body)}
      </ReactMarkdown>
    </article>
  );
}

function StructureBody({ data, onSelect }: { data: MemoryPageResponse; onSelect: (path: string) => void }) {
  return (
    <div className="space-y-6">
      <section>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-text-tertiary">Ontology types ({data.types.length})</h3>
        <div className="space-y-2">
          {data.types.map((type) => (
            <div key={type.iri} className="rounded-lg border border-border bg-surface p-3">
              <p className="text-sm font-medium text-text-primary">{type.label}</p>
              <p className="mt-1 break-all font-mono text-[10px] text-text-tertiary">{type.iri}</p>
              {type.ancestors.length > 0 && <p className="mt-2 text-xs text-text-secondary">is a {type.ancestors.map((ancestor) => ancestor.label).join(" → ")}</p>}
            </div>
          ))}
          {data.types.length === 0 && <p className="text-sm text-text-tertiary">No ontology type</p>}
        </div>
      </section>

      <section>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-text-tertiary">Attributes ({data.attributes.length})</h3>
        <div className="overflow-hidden rounded-lg border border-border bg-surface">
          {data.attributes.map((attribute, index) => (
            <div key={attribute.property} className={`p-3 ${index > 0 ? "border-t border-border" : ""}`}>
              <div className="flex items-start justify-between gap-3">
                <p className="text-sm font-medium text-text-primary">{attribute.label}</p>
                <span className="rounded bg-surface-tertiary px-1.5 py-0.5 font-mono text-[10px] text-text-tertiary">{attribute.datatype}</span>
              </div>
              <pre className="mt-1 whitespace-pre-wrap break-words font-sans text-xs text-text-secondary">{valueText(attribute.value)}</pre>
              <p className="mt-1 break-all font-mono text-[10px] text-text-tertiary">{attribute.property}</p>
            </div>
          ))}
          {data.attributes.length === 0 && <p className="p-3 text-sm text-text-tertiary">No structured attributes</p>}
        </div>
      </section>

      {relationList("Outgoing relations", data.outgoingRelations, data.page.path, onSelect)}
      {relationList("Incoming relations", data.incomingRelations, data.page.path, onSelect)}
    </div>
  );
}

function MemoryBody({ data }: { data: MemoryPageResponse }) {
  return (
    <div className="space-y-3">
      <p className="text-sm text-text-secondary">The atomic memories and evidence used to construct this page.</p>
      {data.memories.map((memory) => {
        const destination = evidenceDestination(memory);
        return (
          <section key={memory.id} className="rounded-xl border border-border bg-surface p-4">
            <div className="flex flex-wrap items-center gap-2">
              <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-secondary">{memory.kind}</span>
              {memory.disposition !== "none" && (
                <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-secondary">{memory.disposition}</span>
              )}
              <span className="text-[11px] text-text-tertiary">{new Date(memory.created_at).toLocaleString()}</span>
            </div>
            <p className="mt-3 whitespace-pre-wrap text-sm leading-5 text-text-primary">{memory.content}</p>
            {memory.comment && <p className="mt-2 text-xs italic text-text-secondary">{memory.comment}</p>}
            {memory.relations.length > 0 && (
              <div className="mt-3">
                <p className="text-xs font-medium text-text-tertiary">Relations</p>
                <pre className="mt-1 overflow-x-auto whitespace-pre-wrap text-[11px] text-text-secondary">{JSON.stringify(memory.relations, null, 2)}</pre>
              </div>
            )}
            {memory.episode && (
              <details className="mt-3 text-xs text-text-secondary">
                <summary className="cursor-pointer text-text-tertiary">Episode</summary>
                <pre className="mt-2 overflow-x-auto whitespace-pre-wrap">{JSON.stringify(memory.episode, null, 2)}</pre>
              </details>
            )}
            <div className="mt-3 border-t border-border pt-3">
              <div className="flex items-center justify-between">
                <p className="text-xs font-medium text-text-tertiary">Evidence ({memory.evidence.length})</p>
                {destination && (
                  <Link href={destination} className="flex items-center gap-1 text-xs text-accent hover:underline">
                    Open source <ArrowTopRightOnSquareIcon className="h-3.5 w-3.5" />
                  </Link>
                )}
              </div>
              <pre className="mt-2 overflow-x-auto whitespace-pre-wrap text-[11px] text-text-secondary">{JSON.stringify(memory.evidence, null, 2)}</pre>
            </div>
            <details className="mt-3 text-[11px] text-text-secondary">
              <summary className="cursor-pointer text-text-tertiary">Lifecycle and identifiers</summary>
              <pre className="mt-2 overflow-x-auto whitespace-pre-wrap">{JSON.stringify({ id: memory.id, ended_at: memory.ended_at, erroneous_at: memory.erroneous_at }, null, 2)}</pre>
            </details>
          </section>
        );
      })}
      {data.memories.length === 0 && <p className="rounded-lg border border-border bg-surface p-3 text-sm text-text-tertiary">No source memories are attached to this page.</p>}
    </div>
  );
}

function PageInspector({
  selectedPath,
  tab,
  onTab,
  onSelect,
  onBackToSearch,
  searchQuery,
}: Pick<MemoryInspectorProps, "selectedPath" | "tab" | "onTab" | "onSelect" | "onBackToSearch" | "searchQuery">) {
  const [data, setData] = useState<MemoryPageResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setData(null);
    setError(null);
    getMemoryPage(selectedPath)
      .then((result) => { if (!cancelled) setData(result); })
      .catch((reason) => { if (!cancelled) setError(reason instanceof Error ? reason.message : "Could not load page"); });
    return () => { cancelled = true; };
  }, [selectedPath]);

  if (error) return <div className="p-4 text-sm text-error-text">{error}</div>;
  if (!data) return <div className="p-4 text-sm text-text-secondary">Loading page…</div>;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="border-b border-border px-4 pb-3 pt-4 pr-14">
        <button onClick={onBackToSearch} className="mb-2 flex items-center gap-1 text-xs text-text-secondary hover:text-text-primary">
          <ArrowLeftIcon className="h-3.5 w-3.5" /> {searchQuery.trim() ? "Search results" : "Browse memory"}
        </button>
        <h2 className="text-xl font-semibold text-text-primary">{data.page.name}</h2>
        <p className="mt-1 font-mono text-[11px] text-text-tertiary">{data.page.path}</p>
        <p className="mt-2 text-sm leading-5 text-text-secondary">{data.page.description}</p>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {data.types.map((type) => <span key={type.iri} className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-secondary">{type.label}</span>)}
          {data.page.origin === "external" && (
            <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-secondary">External</span>
          )}
        </div>
      </div>
      <div className="flex border-b border-border px-4">
        {(["page", "structure", "memory"] as const).map((item) => (
          <button
            key={item}
            onClick={() => onTab(item)}
            className={`border-b-2 px-3 py-2.5 text-sm capitalize ${tab === item ? "border-accent text-text-primary" : "border-transparent text-text-secondary hover:text-text-primary"}`}
          >
            {item}{item === "memory" ? ` (${data.memories.length})` : ""}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {tab === "page" && <PageBody data={data} onSelect={onSelect} />}
        {tab === "structure" && <StructureBody data={data} onSelect={onSelect} />}
        {tab === "memory" && <MemoryBody data={data} />}
      </div>
    </div>
  );
}

export function MemoryInspector(props: MemoryInspectorProps) {
  return (
    <AnimatePresence>
      {props.open && (
        <motion.aside
          initial={props.mobile ? { y: "100%" } : { x: "100%" }}
          animate={props.mobile ? { y: 0 } : { x: 0 }}
          exit={props.mobile ? { y: "100%" } : { x: "100%" }}
          transition={{ duration: 0.22, ease: "easeOut" }}
          className={props.mobile
            ? "absolute inset-x-0 bottom-0 z-30 h-[78%] overflow-hidden rounded-t-2xl border-t border-border bg-surface-secondary shadow-2xl"
            : "absolute inset-y-0 right-0 z-30 w-[min(460px,42vw)] overflow-hidden border-l border-border bg-surface-secondary shadow-xl"}
        >
          {props.mobile && <div className="mx-auto mt-2 h-1 w-10 rounded-full bg-border" />}
          <button onClick={props.onClose} className="absolute right-3 top-3 z-10 rounded-lg bg-surface-secondary p-2 text-text-secondary hover:bg-surface-tertiary hover:text-text-primary" aria-label="Close inspector">
            <XMarkIcon className="h-5 w-5" />
          </button>
          <div className={props.mobile ? "h-[calc(100%-12px)] pt-3" : "h-full"}>
            {props.searchMode ? (
              <SearchResults
                query={props.searchQuery}
                results={props.searchResults}
                browsablePages={props.browsablePages}
                loading={props.searchLoading}
                onSelect={(path) => props.onSelect(path, true)}
              />
            ) : (
              <PageInspector
                selectedPath={props.selectedPath}
                tab={props.tab}
                onTab={props.onTab}
                onSelect={props.onSelect}
                onBackToSearch={props.onBackToSearch}
                searchQuery={props.searchQuery}
              />
            )}
          </div>
        </motion.aside>
      )}
    </AnimatePresence>
  );
}
