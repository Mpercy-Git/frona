/**
 * Parses syd(box) JSON event lines out of shell tool output.
 *
 * Events come from the sandbox's structured-log macro, e.g.
 *
 *   {"ctx":"access","cap":"net/connect","act":"deny","addr":"1.2.3.4!80", …}
 *
 * The schema varies per capability (`net/*` uses `addr`+`ipv`; file ops use
 * `path`; exec uses `path` or `argv`; etc.), so we detect events by the set of
 * always-present fields (`ctx`, `act`, `sys`, `cap`) and pick a human-friendly
 * target from the rest. Events that allowed/warned the operation are ignored —
 * only deny-class actions surface (deny, kill, stop, abort, panic, exit, filter).
 *
 * Surviving events are deduplicated by `(cap, act, target)` since syd typically
 * logs once per resolved IP, retry, and pid.
 */

/** Sandbox `act` values that mean the operation did NOT proceed. */
const DENY_ACTS: ReadonlySet<string> = new Set([
  "deny", "kill", "stop", "abort", "panic", "exit", "filter",
]);

export type Severity = "low" | "high" | "critical";

const SEVERITY: Record<string, Severity> = {
  filter: "low",
  deny: "high",
  stop: "high",
  kill: "critical",
  abort: "critical",
  panic: "critical",
  exit: "critical",
};

export interface SandboxEvent {
  cap: string;
  act: string;
  /** Best human-readable target — addr (net), path (file/exec), or sys fallback. */
  target: string;
  /** sydbox-supplied fix hint, if any. */
  tip?: string;
  /** Severity derived from `act`. */
  severity: Severity;
}

export interface ParsedShellOutput {
  events: SandboxEvent[];
  remainingText: string;
}

function isStringField(o: Record<string, unknown>, k: string): boolean {
  return typeof o[k] === "string" && (o[k] as string).length > 0;
}

function normalizeAddr(addr: string): string {
  // syd uses '!' as the host/port separator (`1.2.3.4!80`); render with a colon.
  return addr.replace(/!(?=\d+$)/, ":");
}

function pickTarget(o: Record<string, unknown>): string {
  if (typeof o.addr === "string") return normalizeAddr(o.addr);
  if (typeof o.path === "string") return o.path;
  if (Array.isArray(o.argv) && o.argv.length > 0 && typeof o.argv[0] === "string") {
    return o.argv[0];
  }
  if (typeof o.cmd === "string") return o.cmd;
  if (typeof o.sys === "string") return o.sys;
  return "";
}

type Classified =
  | { kind: "not-event" }
  | { kind: "event-non-deny" }
  | { kind: "event-deny"; event: SandboxEvent };

function classifyJson(jsonText: string): Classified {
  let obj: unknown;
  try {
    obj = JSON.parse(jsonText);
  } catch {
    return { kind: "not-event" };
  }
  if (!obj || typeof obj !== "object" || Array.isArray(obj)) return { kind: "not-event" };
  const o = obj as Record<string, unknown>;
  if (
    !isStringField(o, "ctx") ||
    !isStringField(o, "act") ||
    !isStringField(o, "sys") ||
    !isStringField(o, "cap")
  ) {
    return { kind: "not-event" };
  }
  const act = o.act as string;
  if (!DENY_ACTS.has(act)) return { kind: "event-non-deny" };
  return {
    kind: "event-deny",
    event: {
      cap: o.cap as string,
      act,
      target: pickTarget(o),
      tip: typeof o.tip === "string" ? o.tip : undefined,
      severity: SEVERITY[act] ?? "high",
    },
  };
}

/**
 * Find the position of the `}` that matches the `{` at `start`. Respects
 * JSON string literals (so `{` / `}` inside `"..."` don't count) and
 * backslash-escaped chars. Returns -1 if no match.
 */
function findMatchingBrace(text: string, start: number): number {
  let depth = 0;
  let inString = false;
  let escape = false;
  for (let i = start; i < text.length; i++) {
    const c = text[i];
    if (escape) {
      escape = false;
      continue;
    }
    if (c === "\\") {
      escape = true;
      continue;
    }
    if (inString) {
      if (c === '"') inString = false;
      continue;
    }
    if (c === '"') {
      inString = true;
      continue;
    }
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

export function parseShellOutput(text: string): ParsedShellOutput {
  if (!text) return { events: [], remainingText: "" };

  const events: SandboxEvent[] = [];
  let out = "";
  let i = 0;

  // Character-level scanner: walk forward, find each `{"` that starts a
  // JSON-looking object, brace-match to its end, classify, and either drop it
  // or splice it back into the output. Required because curl/wget output uses
  // `\r` to redraw progress, so sandbox events can land mid-line with no
  // preceding newline.
  while (i < text.length) {
    const next = text.indexOf('{"', i);
    if (next < 0) {
      out += text.slice(i);
      break;
    }
    out += text.slice(i, next);

    const end = findMatchingBrace(text, next);
    if (end < 0) {
      // Unmatched brace — keep the `{` literal and continue scanning past it.
      out += text[next];
      i = next + 1;
      continue;
    }

    const candidate = text.slice(next, end + 1);
    const c = classifyJson(candidate);
    if (c.kind === "event-deny") {
      events.push(c.event);
    } else if (c.kind === "event-non-deny") {
      // Recognized syd event we choose not to surface — still strip it.
    } else {
      // Not a syd event — keep the JSON in the output verbatim.
      out += candidate;
    }
    i = end + 1;
  }

  // Dedup by (cap, act, target). Preserve insertion order; keep the first tip we see.
  const seen = new Map<string, SandboxEvent>();
  for (const ev of events) {
    const key = `${ev.cap}|${ev.act}|${ev.target}`;
    const prev = seen.get(key);
    if (!prev) {
      seen.set(key, ev);
    } else if (!prev.tip && ev.tip) {
      seen.set(key, { ...prev, tip: ev.tip });
    }
  }

  // Removing JSON often leaves runs of blank lines (each event line becomes
  // empty after the JSON is stripped). Collapse 3+ consecutive newlines to 2
  // so paragraph breaks survive but stripped-event lines don't multiply.
  const cleaned = out.replace(/\n{3,}/g, "\n\n").trimEnd();

  return {
    events: Array.from(seen.values()),
    remainingText: cleaned,
  };
}

/** Highest severity among the given events. Returns null for an empty list. */
export function bestSeverity(events: SandboxEvent[]): Severity | null {
  if (events.length === 0) return null;
  if (events.some((e) => e.severity === "critical")) return "critical";
  if (events.some((e) => e.severity === "high")) return "high";
  return "low";
}

/** Group events by `cap|act` for rendering. */
export function groupEvents(events: SandboxEvent[]): Map<string, SandboxEvent[]> {
  const m = new Map<string, SandboxEvent[]>();
  for (const ev of events) {
    const key = `${ev.cap}|${ev.act}`;
    const list = m.get(key);
    if (list) list.push(ev);
    else m.set(key, [ev]);
  }
  return m;
}
