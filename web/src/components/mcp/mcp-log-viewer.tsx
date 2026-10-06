"use client";

import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import {
  ArrowDownTrayIcon,
  ClipboardDocumentIcon,
  CommandLineIcon,
  MagnifyingGlassIcon,
  PauseIcon,
  PlayIcon,
  TrashIcon,
} from "@heroicons/react/24/outline";
import { api, API_URL, ensureAccessToken } from "@/lib/api-client";
import { SectionHeader } from "@/components/settings/field";

/** Cap the in-memory buffer so a chatty server can't grow the page without bound. */
export const MAX_LINES = 5000;
/** How long to wait before reopening a stream the server or network closed. */
const RECONNECT_DELAY_MS = 3000;

export type LineLevel = "error" | "warn" | "info";

/**
 * MCP servers write free-form stderr, so a level is a guess from the words a
 * line carries rather than a field it reports.
 */
export function classifyLine(line: string): LineLevel {
  if (/\b(error|err|fatal|panic|exception|traceback|failed)\b/i.test(line)) return "error";
  if (/\b(warn|warning|deprecated)\b/i.test(line)) return "warn";
  return "info";
}

export type SseFrame = { kind: "line"; text: string } | { kind: "reset" };

/**
 * Split the SSE text received so far into frames, returning the unterminated
 * remainder to carry into the next chunk. A `reset` event means the log was
 * cleared and the stream is starting over from its top.
 */
export function parseSseChunk(buffer: string): { frames: SseFrame[]; rest: string } {
  const frames: SseFrame[] = [];
  const blocks = buffer.split("\n\n");
  const rest = blocks.pop() ?? "";
  for (const block of blocks) {
    let event = "message";
    const data: string[] = [];
    for (const raw of block.split("\n")) {
      if (raw.startsWith("event:")) event = raw.slice(6).trim();
      else if (raw.startsWith("data:")) data.push(raw.slice(raw.startsWith("data: ") ? 6 : 5));
    }
    if (event === "reset") frames.push({ kind: "reset" });
    else if (data.length > 0) frames.push({ kind: "line", text: data.join("\n") });
  }
  return { frames, rest };
}

const LEVEL_CLASS: Record<LineLevel, string> = {
  error: "text-error-text",
  warn: "text-warning",
  info: "text-text-primary",
};

function capped(lines: string[]): string[] {
  return lines.length > MAX_LINES ? lines.slice(lines.length - MAX_LINES) : lines;
}

interface McpLogViewerProps {
  serverId: string;
  /** Used to reopen the stream promptly when the server is (re)started. */
  serverStatus: string;
}

export function McpLogViewer({ serverId, serverStatus }: McpLogViewerProps) {
  const [lines, setLines] = useState<string[]>([]);
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(true);
  const [paused, setPaused] = useState(false);
  const [follow, setFollow] = useState(true);
  const [query, setQuery] = useState("");
  const [issuesOnly, setIssuesOnly] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [clearing, setClearing] = useState(false);

  // Lines that arrive while paused wait here instead of being dropped, so
  // resuming shows everything the server wrote in the meantime.
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const heldRef = useRef<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const controller = new AbortController();
    let retryTimer: ReturnType<typeof setTimeout> | undefined;

    const connect = async () => {
      // `ensureAccessToken` rather than the cached token: after an expiry the
      // cached one is dead, and a log pane that silently stays empty is a
      // worse symptom than the request that renews it.
      const tokenResult = await ensureAccessToken();
      const headers: Record<string, string> = {};
      if (tokenResult.ok) headers["Authorization"] = `Bearer ${tokenResult.token}`;

      try {
        const res = await fetch(`${API_URL}/api/mcp/servers/${serverId}/logs/stream`, {
          headers,
          signal: controller.signal,
          credentials: "include",
        });
        if (!res.ok || !res.body) return;

        // Each connection replays the log's tail, so what came before it is
        // replaced rather than appended to.
        heldRef.current = [];
        setLines([]);
        setConnected(true);
        setLoading(false);

        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          const { frames, rest } = parseSseChunk(buffer);
          buffer = rest;

          let incoming: string[] = [];
          let reset = false;
          for (const f of frames) {
            if (f.kind === "reset") {
              reset = true;
              incoming = [];
              heldRef.current = [];
            } else {
              incoming.push(f.text);
            }
          }
          if (!reset && incoming.length === 0) continue;
          if (pausedRef.current) {
            heldRef.current = capped(heldRef.current.concat(incoming));
            if (reset) setLines([]);
          } else {
            setLines((prev) => capped((reset ? [] : prev).concat(incoming)));
          }
        }
      } catch {
        // aborted or connection lost
      } finally {
        setConnected(false);
        setLoading(false);
        if (!controller.signal.aborted) {
          retryTimer = setTimeout(connect, RECONNECT_DELAY_MS);
        }
      }
    };

    setLoading(true);
    connect();
    return () => {
      controller.abort();
      if (retryTimer) clearTimeout(retryTimer);
    };
    // A start or restart reopens the stream so the new run's output shows
    // without waiting out the reconnect delay.
  }, [serverId, serverStatus]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q && !issuesOnly) return lines;
    return lines.filter((l) => {
      if (issuesOnly && classifyLine(l) === "info") return false;
      return !q || l.toLowerCase().includes(q);
    });
  }, [lines, query, issuesOnly]);

  useEffect(() => {
    if (follow && !paused) endRef.current?.scrollIntoView({ block: "end" });
  }, [visible, follow, paused]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
    setFollow(atBottom);
  }, []);

  const togglePause = () => {
    if (paused && heldRef.current.length > 0) {
      const held = heldRef.current;
      heldRef.current = [];
      setLines((prev) => capped(prev.concat(held)));
    }
    setPaused((p) => !p);
  };

  const flash = (message: string) => {
    setNotice(message);
    setTimeout(() => setNotice(null), 2500);
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(visible.join("\n"));
      flash(`Copied ${visible.length} line${visible.length === 1 ? "" : "s"}`);
    } catch {
      flash("Copy failed");
    }
  };

  const download = () => {
    const blob = new Blob([visible.join("\n") + "\n"], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `mcp-${serverId}-${new Date().toISOString().replace(/[:.]/g, "-")}.log`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const clear = async () => {
    if (!confirm("Clear this server's log? Earlier output will be deleted.")) return;
    setClearing(true);
    try {
      await api.delete(`/api/mcp/servers/${serverId}/logs`);
      heldRef.current = [];
      setLines([]);
    } catch (e: unknown) {
      flash(e instanceof Error ? e.message : "Clear failed");
    } finally {
      setClearing(false);
    }
  };

  const filtered = query.trim() !== "" || issuesOnly;
  const toolButton =
    "inline-flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-xs text-text-secondary hover:bg-surface-tertiary disabled:opacity-50 transition";

  return (
    <div className="space-y-6">
      <SectionHeader title="Logs" description="Server stderr and install output" icon={CommandLineIcon} />

      <div className="rounded-xl border border-border bg-surface-secondary overflow-hidden">
        {/* Toolbar */}
        <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
          <span className="flex items-center gap-1.5 text-xs text-text-tertiary">
            <span className={`h-2 w-2 rounded-full ${connected ? "bg-green-500" : "bg-text-tertiary"}`} />
            {connected ? "Streaming" : loading ? "Connecting…" : "Reconnecting…"}
          </span>

          <div className="relative min-w-[140px] flex-1">
            <MagnifyingGlassIcon className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-text-tertiary" />
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter lines"
              aria-label="Filter log lines"
              className="w-full rounded-lg border border-border bg-surface py-1 pl-7 pr-2 text-xs text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
            />
          </div>

          <label className="flex items-center gap-1.5 text-xs text-text-secondary">
            <input
              type="checkbox"
              checked={issuesOnly}
              onChange={(e) => setIssuesOnly(e.target.checked)}
              className="h-3.5 w-3.5 rounded border-border text-accent focus:ring-accent"
            />
            Errors &amp; warnings
          </label>

          <div className="ml-auto flex items-center gap-2">
            <button type="button" onClick={togglePause} className={toolButton}>
              {paused ? <PlayIcon className="h-3.5 w-3.5" /> : <PauseIcon className="h-3.5 w-3.5" />}
              {paused ? "Resume" : "Pause"}
            </button>
            <button type="button" onClick={copy} disabled={visible.length === 0} className={toolButton} title="Copy visible lines">
              <ClipboardDocumentIcon className="h-3.5 w-3.5" />
              Copy
            </button>
            <button type="button" onClick={download} disabled={visible.length === 0} className={toolButton} title="Download visible lines">
              <ArrowDownTrayIcon className="h-3.5 w-3.5" />
              Download
            </button>
            <button type="button" onClick={clear} disabled={clearing} className={toolButton} title="Delete the server's log file contents">
              <TrashIcon className="h-3.5 w-3.5" />
              Clear
            </button>
          </div>
        </div>

        {/* Log body */}
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className="h-[520px] overflow-y-auto bg-surface px-3 py-2 font-mono text-xs leading-5"
        >
          {loading && lines.length === 0 ? (
            <div className="flex items-center justify-center py-12">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
            </div>
          ) : visible.length === 0 ? (
            <p className="py-12 text-center text-text-tertiary">
              {filtered && lines.length > 0
                ? "No lines match the filter."
                : "No logs yet. Start the server to see its output."}
            </p>
          ) : (
            visible.map((l, i) => (
              <div key={i} className={`whitespace-pre-wrap break-words ${LEVEL_CLASS[classifyLine(l)]}`}>
                {l}
              </div>
            ))
          )}
          <div ref={endRef} />
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between gap-2 border-t border-border px-3 py-1.5 text-[11px] text-text-tertiary">
          <span>
            {filtered ? `${visible.length} of ${lines.length}` : lines.length} line{lines.length === 1 ? "" : "s"}
            {paused ? ` · paused${heldRef.current.length > 0 ? ` (${heldRef.current.length} new)` : ""}` : ""}
            {notice ? ` · ${notice}` : ""}
          </span>
          {!follow && (
            <button
              type="button"
              onClick={() => { setFollow(true); endRef.current?.scrollIntoView({ block: "end" }); }}
              className="text-accent hover:underline"
            >
              Jump to latest
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
