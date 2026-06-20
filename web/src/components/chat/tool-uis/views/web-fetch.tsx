"use client";

import { useCallback, useState } from "react";
import { CheckIcon, ClipboardDocumentIcon } from "@heroicons/react/24/outline";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { CodeBlock } from "@/components/ui/code-block";
import { cn } from "@/lib/utils";
import { ToolRow } from "./tool-row";
import type { ToolView } from "./types";

function prettyUrl(url: string): string {
  try {
    const u = new URL(url);
    const path = u.pathname === "/" ? "" : u.pathname;
    return `${u.host}${path}${u.search}`;
  } catch {
    return url;
  }
}

function UrlBlock({ url }: { url: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    },
    [url],
  );

  const handleOpen = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      window.open(
        url,
        "_blank",
        "noopener,noreferrer,width=1280,height=900,scrollbars=yes,resizable=yes",
      );
    },
    [url],
  );

  return (
    <div className="group/url not-prose relative">
      <a
        href={url}
        target="_blank"
        rel="noopener noreferrer"
        onClick={handleOpen}
        className="block rounded-lg bg-surface-nav p-4 pr-12 text-[0.8125rem] font-mono text-accent hover:underline break-all"
      >
        {url}
      </a>
      <button
        type="button"
        onClick={handleCopy}
        aria-label="Copy URL"
        className={cn(
          "absolute top-2 right-2 flex items-center justify-center h-7 w-7 rounded-md",
          "bg-surface-tertiary/80 text-text-secondary",
          "hover:text-text-primary hover:bg-surface-tertiary",
          "transition-all opacity-0 group-hover/url:opacity-100 focus-visible:opacity-100",
        )}
      >
        {copied ? (
          <CheckIcon className="h-4 w-4 text-[#4fd1c5]" />
        ) : (
          <ClipboardDocumentIcon className="h-4 w-4" />
        )}
      </button>
    </div>
  );
}

export const WebFetchView: ToolView = ({
  args,
  result,
  status,
  isExpanded,
  onToggle,
}) => {
  const a = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
  const url = typeof a.url === "string" ? a.url : "";
  const subtitle = url ? prettyUrl(url) : null;
  const text =
    typeof result === "string"
      ? result
      : result !== undefined
        ? JSON.stringify(result, null, 2)
        : "";

  const errorText =
    status?.type === "incomplete"
      ? (() => {
          const e = (status as { error?: unknown }).error;
          return e == null
            ? null
            : typeof e === "string"
              ? e
              : JSON.stringify(e);
        })()
      : null;

  return (
    <ToolRow status={status} expandable={text.length > 0 || url.length > 0}>
      <ToolRow.Header onToggle={onToggle} isExpanded={isExpanded}>
        <ToolRow.Title>Web Fetch</ToolRow.Title>
        <ToolRow.Subtitle>{subtitle}</ToolRow.Subtitle>
      </ToolRow.Header>

      <ToolRow.Error>
        <div className="flex flex-col gap-2">
          {url && <UrlBlock url={url} />}
          <div className="text-xs px-3 pb-3">
            <p className="font-semibold text-danger">Fetch failed</p>
            {errorText && errorText !== text && (
              <pre className="whitespace-pre-wrap text-text-tertiary mt-1">
                {errorText}
              </pre>
            )}
            {text && (
              <pre className="whitespace-pre-wrap text-text-tertiary mt-1">
                {text}
              </pre>
            )}
          </div>
        </div>
      </ToolRow.Error>

      <ToolRow.Body isExpanded={isExpanded} unstyled>
        <div className="flex flex-col gap-2">
          {url && <UrlBlock url={url} />}
          {text && (
            <div className="prose prose-sm max-w-none px-3 pb-3 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 text-text-primary prose-headings:text-text-primary prose-strong:text-text-primary prose-a:text-accent prose-code:text-text-primary prose-code:before:content-none prose-code:after:content-none prose-blockquote:text-text-secondary prose-blockquote:border-border prose-hr:border-border">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={{
                  code({ className, children, ...props }) {
                    const match = /language-(\w+)/.exec(className || "");
                    const code = String(children).replace(/\n$/, "");
                    if (match) return <CodeBlock code={code} language={match[1]} />;
                    return (
                      <code className={className} {...props}>
                        {children}
                      </code>
                    );
                  },
                }}
              >
                {text}
              </ReactMarkdown>
            </div>
          )}
        </div>
      </ToolRow.Body>
    </ToolRow>
  );
};
