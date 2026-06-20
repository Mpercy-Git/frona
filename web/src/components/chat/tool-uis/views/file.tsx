"use client";

import type { ReactNode } from "react";
import { CodeBlock } from "@/components/ui/code-block";
import { ToolRow } from "./tool-row";
import type { ToolView } from "./types";

const EXT_TO_LANG: Record<string, string> = {
  ts: "ts", tsx: "tsx", js: "js", jsx: "jsx", mjs: "js", cjs: "js",
  py: "python", rs: "rust", go: "go", rb: "ruby", php: "php",
  java: "java", kt: "kotlin", swift: "swift", scala: "scala",
  c: "c", h: "c", cpp: "cpp", cc: "cpp", hpp: "cpp", cxx: "cpp",
  cs: "csharp", m: "objc", mm: "objc",
  sh: "bash", bash: "bash", zsh: "bash", fish: "fish",
  md: "markdown", mdx: "mdx", txt: "text", log: "text",
  json: "json", yaml: "yaml", yml: "yaml", toml: "toml", xml: "xml",
  html: "html", htm: "html", css: "css", scss: "scss", sass: "sass",
  sql: "sql", graphql: "graphql", gql: "graphql",
  lua: "lua", dart: "dart", r: "r", jl: "julia",
  vue: "vue", svelte: "svelte",
  dockerfile: "docker",
};

function inferLanguageFromPath(path: string | null | undefined): string {
  if (!path) return "text";
  const base = path.split("/").pop() ?? path;
  if (base.toLowerCase() === "dockerfile") return "docker";
  const dot = base.lastIndexOf(".");
  if (dot < 0) return "text";
  const ext = base.slice(dot + 1).toLowerCase();
  return EXT_TO_LANG[ext] ?? "text";
}

function getSubtitle(toolName: string, args: Record<string, unknown>): string | null {
  if (toolName === "glob" || toolName === "grep") {
    const pattern = typeof args.pattern === "string" ? args.pattern : null;
    const scope = typeof args.path === "string" ? args.path : null;
    if (pattern && scope && scope !== ".") return `${pattern} in ${scope}`;
    return pattern;
  }
  return typeof args.path === "string" ? args.path : null;
}

function globLiteralsToRegex(pattern: string): RegExp | null {
  try {
    const literals = pattern
      .split(/\*\*|\*|\?|\//)
      .map((p) => p.trim())
      .filter((p) => p.length >= 2);
    if (literals.length === 0) return null;
    const escaped = [...new Set(literals)].map((p) =>
      p.replace(/[.+^${}()|[\]\\]/g, "\\$&"),
    );
    return new RegExp(`(?:${escaped.join("|")})`, "g");
  } catch {
    return null;
  }
}

function highlightMatches(text: string, re: RegExp | null): ReactNode {
  if (!re) return text;
  const parts: ReactNode[] = [];
  let last = 0;
  re.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    parts.push(
      <span key={`m-${m.index}`} className="text-info font-semibold">
        {m[0]}
      </span>,
    );
    last = m.index + m[0].length;
    if (m[0].length === 0) re.lastIndex++;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts.length > 0 ? parts : text;
}

const TOOL_DISPLAY: Record<string, string> = {
  read: "Read",
  write: "Write",
  edit: "Edit",
  glob: "Glob",
  grep: "Grep",
};

export const FileView: ToolView = ({
  toolName,
  args,
  result,
  status,
  isExpanded,
  onToggle,
}) => {
  const a = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
  const path = typeof a.path === "string" ? a.path : null;
  const subtitle = getSubtitle(toolName, a);

  return (
    <ToolRow status={status} expandable>
      <ToolRow.Header onToggle={onToggle} isExpanded={isExpanded}>
        <ToolRow.Title>{TOOL_DISPLAY[toolName] ?? toolName}</ToolRow.Title>
        <ToolRow.Subtitle>{subtitle}</ToolRow.Subtitle>
      </ToolRow.Header>

      <ToolRow.Body isExpanded={isExpanded} unstyled>
        <FileExpanded
          toolName={toolName}
          args={a}
          path={path}
          result={result}
          subtitle={subtitle}
        />
      </ToolRow.Body>
    </ToolRow>
  );
};

function FileExpanded({
  toolName,
  args,
  path,
  result,
  subtitle,
}: {
  toolName: string;
  args: Record<string, unknown>;
  path: string | null;
  result: unknown;
  subtitle: string | null;
}) {
  const lang = inferLanguageFromPath(path);
  const showPath = path && path !== subtitle;

  if (toolName === "write") {
    const content = typeof args.content === "string" ? args.content : "";
    return (
      <div className="flex flex-col gap-2">
        {showPath && <p className="font-mono text-xs text-text-tertiary px-3 pt-2">{path}</p>}
        <CodeBlock code={content} language={lang} lineNumbers />
      </div>
    );
  }

  if (toolName === "edit") {
    const text = typeof result === "string" ? result : "";
    const marker = "Surrounding context:\n";
    const markerIdx = text.indexOf(marker);
    const summary = markerIdx >= 0 ? text.slice(0, markerIdx).trim() : null;
    const snippet = markerIdx >= 0 ? text.slice(markerIdx + marker.length) : null;
    return (
      <div className="flex flex-col gap-2">
        {showPath && <p className="font-mono text-xs text-text-tertiary px-3 pt-2">{path}</p>}
        {summary && (
          <p className="px-3 pt-2 text-xs text-text-tertiary">{summary}</p>
        )}
        {snippet !== null ? (
          <CodeBlock code={snippet} language={lang} />
        ) : (
          <pre className="whitespace-pre-wrap text-text-secondary text-xs overflow-x-auto px-3 py-2">
            {text}
          </pre>
        )}
      </div>
    );
  }

  if (toolName === "read") {
    const text = typeof result === "string" ? result : null;
    return (
      <div className="flex flex-col gap-2">
        {showPath && <p className="font-mono text-xs text-text-tertiary px-3 pt-2">{path}</p>}
        {text !== null && <CodeBlock code={text} language={lang} lineNumbers />}
      </div>
    );
  }

  // glob / grep — line listings with optional match highlighting
  const text =
    typeof result === "string"
      ? result
      : result !== undefined
        ? JSON.stringify(result, null, 2)
        : "";

  const truncMatch = text.match(/\n\n\[truncated[^\]]+\]$/);
  const trailing = truncMatch?.[0]?.trim() ?? null;
  const body = trailing ? text.slice(0, text.length - truncMatch![0].length) : text;
  const lines = body.split("\n").filter((l) => l.length > 0);

  const pattern = typeof args.pattern === "string" ? args.pattern : null;
  let matchRe: RegExp | null = null;
  if (toolName === "grep" && pattern) {
    try {
      matchRe = new RegExp(pattern, "g");
    } catch {
      matchRe = null;
    }
  } else if (toolName === "glob" && pattern) {
    matchRe = globLiteralsToRegex(pattern);
  }

  const grepLine = /^(.+?):(\d+):(.*)$/;
  return (
    <pre className="font-mono text-xs text-text-secondary bg-surface-nav rounded-b-md overflow-auto p-3 m-0 leading-relaxed">
      {lines.map((line, i) => {
        const m = toolName === "grep" ? line.match(grepLine) : null;
        if (m) {
          return (
            <div key={i} className="flex gap-3 whitespace-nowrap">
              <span className="text-text-tertiary shrink-0">
                {m[1]}:{m[2]}
              </span>
              <span>{highlightMatches(m[3], matchRe)}</span>
            </div>
          );
        }
        return (
          <div key={i} className="whitespace-nowrap">
            {highlightMatches(line, matchRe)}
          </div>
        );
      })}
      {trailing && (
        <div className="mt-2 italic text-text-tertiary whitespace-nowrap">
          {trailing}
        </div>
      )}
    </pre>
  );
}
