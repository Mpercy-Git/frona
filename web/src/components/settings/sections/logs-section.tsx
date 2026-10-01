"use client";

import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { CommandLineIcon, PauseIcon, PlayIcon, TrashIcon } from "@heroicons/react/24/outline";
import { API_URL, ensureAccessToken } from "@/lib/api-client";
import { SectionHeader } from "../field";

interface LogLine {
  timestamp: string;
  level: string;
  target: string;
  message: string;
}

/** Cap the in-memory buffer so a long-lived viewer can't grow without bound. */
const MAX_LINES = 2000;

const LEVEL_ORDER: Record<string, number> = { ERROR: 0, WARN: 1, INFO: 2, DEBUG: 3, TRACE: 4 };
const LEVELS = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"] as const;
type LevelFilter = (typeof LEVELS)[number];

const LEVEL_CLASS: Record<string, string> = {
  ERROR: "text-error-text",
  WARN: "text-warning",
  INFO: "text-text-secondary",
  DEBUG: "text-text-tertiary",
  TRACE: "text-text-tertiary",
};

function formatTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleTimeString(undefined, { hour12: false }) + "." +
    String(d.getMilliseconds()).padStart(3, "0");
}

export function LogsSection() {
  const [lines, setLines] = useState<LogLine[]>([]);
  const [connected, setConnected] = useState(false);
  const [paused, setPaused] = useState(false);
  const [levelFilter, setLevelFilter] = useState<LevelFilter>("ALL");
  const [follow, setFollow] = useState(true);

  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const scrollRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const controller = new AbortController();

    (async () => {
      // `ensureAccessToken` rather than the cached token: after an expiry the
      // cached one is dead, and a log pane that silently stays empty is a
      // worse symptom than the request that renews it.
      const tokenResult = await ensureAccessToken();
      const headers: Record<string, string> = {};
      if (tokenResult.ok) headers["Authorization"] = `Bearer ${tokenResult.token}`;

      let res: Response;
      try {
        res = await fetch(`${API_URL}/api/system/logs/stream`, {
          headers,
          signal: controller.signal,
          credentials: "include",
        });
      } catch {
        return;
      }

      if (!res.ok || !res.body) {
        setConnected(false);
        return;
      }
      setConnected(true);

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          const rawLines = buffer.split("\n");
          buffer = rawLines.pop() ?? "";

          const parsed: LogLine[] = [];
          for (const raw of rawLines) {
            if (!raw.startsWith("data: ")) continue;
            try {
              parsed.push(JSON.parse(raw.slice(6)) as LogLine);
            } catch {
              // ignore keep-alive / malformed frames
            }
          }
          if (parsed.length > 0 && !pausedRef.current) {
            setLines((prev) => {
              const next = prev.concat(parsed);
              return next.length > MAX_LINES ? next.slice(next.length - MAX_LINES) : next;
            });
          }
        }
      } catch {
        // aborted or connection lost
      } finally {
        setConnected(false);
      }
    })();

    return () => controller.abort();
  }, []);

  const visible = useMemo(() => {
    if (levelFilter === "ALL") return lines;
    const max = LEVEL_ORDER[levelFilter] ?? 99;
    return lines.filter((l) => (LEVEL_ORDER[l.level] ?? 99) <= max);
  }, [lines, levelFilter]);

  useEffect(() => {
    if (follow && !paused) endRef.current?.scrollIntoView({ block: "end" });
  }, [visible, follow, paused]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
    setFollow(atBottom);
  }, []);

  const clear = useCallback(() => setLines([]), []);

  return (
    <div className="space-y-6">
      <SectionHeader title="Logs" description="Live server log output" icon={CommandLineIcon} />

      <div className="rounded-xl border border-border bg-surface-secondary overflow-hidden">
        {/* Toolbar */}
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <span className="flex items-center gap-1.5 text-xs text-text-tertiary">
            <span className={`h-2 w-2 rounded-full ${connected ? "bg-green-500" : "bg-text-tertiary"}`} />
            {connected ? "Streaming" : "Disconnected"}
          </span>

          <div className="ml-auto flex items-center gap-2">
            <select
              value={levelFilter}
              onChange={(e) => setLevelFilter(e.target.value as LevelFilter)}
              className="rounded-lg border border-border bg-surface px-2 py-1 text-xs text-text-primary focus:border-accent focus:outline-none"
            >
              {LEVELS.map((l) => (
                <option key={l} value={l}>{l === "ALL" ? "All levels" : l}</option>
              ))}
            </select>

            <button
              type="button"
              onClick={() => setPaused((p) => !p)}
              className="inline-flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-xs text-text-secondary hover:bg-surface-tertiary transition"
            >
              {paused ? <PlayIcon className="h-3.5 w-3.5" /> : <PauseIcon className="h-3.5 w-3.5" />}
              {paused ? "Resume" : "Pause"}
            </button>

            <button
              type="button"
              onClick={clear}
              className="inline-flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-xs text-text-secondary hover:bg-surface-tertiary transition"
            >
              <TrashIcon className="h-3.5 w-3.5" />
              Clear
            </button>
          </div>
        </div>

        {/* Log body */}
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className="h-[520px] overflow-y-auto bg-surface px-3 py-2 font-mono text-xs leading-relaxed"
        >
          {visible.length === 0 ? (
            <p className="py-12 text-center text-text-tertiary">
              {connected ? "Waiting for log output…" : "No logs to display."}
            </p>
          ) : (
            visible.map((l, i) => (
              <div key={i} className="flex gap-2 whitespace-pre-wrap break-words py-0.5">
                <span className="shrink-0 text-text-tertiary">{formatTime(l.timestamp)}</span>
                <span className={`shrink-0 w-12 font-semibold ${LEVEL_CLASS[l.level] ?? "text-text-secondary"}`}>
                  {l.level}
                </span>
                <span className="shrink-0 text-accent/70">{l.target}</span>
                <span className="text-text-primary">{l.message}</span>
              </div>
            ))
          )}
          <div ref={endRef} />
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-border px-3 py-1.5 text-[11px] text-text-tertiary">
          <span>{visible.length} line{visible.length === 1 ? "" : "s"}{paused ? " · paused" : ""}</span>
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
