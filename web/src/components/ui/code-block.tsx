"use client";

import { useState, useCallback, useEffect } from "react";
import { codeToHtml } from "shiki";
import { ClipboardDocumentIcon, CheckIcon } from "@heroicons/react/24/outline";
import { CODE_THEME } from "@/lib/theme";
import { cn } from "@/lib/utils";

function CopyButton({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [code]);

  return (
    <button
      onClick={handleCopy}
      className="absolute top-2 right-2 flex items-center justify-center h-7 w-7 rounded-md bg-surface-tertiary/80 text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-all opacity-0 group-hover/code:opacity-100"
    >
      {copied ? (
        <CheckIcon className="h-4 w-4 text-[#4fd1c5]" />
      ) : (
        <ClipboardDocumentIcon className="h-4 w-4" />
      )}
    </button>
  );
}

export function CodeBlock({
  code,
  language,
  lineNumbers = false,
  wrap = false,
}: {
  code: string;
  language?: string;
  lineNumbers?: boolean;
  wrap?: boolean;
}) {
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    codeToHtml(code, {
      lang: language || "text",
      theme: CODE_THEME,
    })
      .then((result) => {
        if (!cancelled) setHtml(result);
      })
      .catch(() => {
        if (!cancelled) setHtml(`<pre><code>${code}</code></pre>`);
      });
    return () => { cancelled = true; };
  }, [code, language]);

  const lineNumberClasses = lineNumbers
    ? "[&_code]:[counter-reset:line] [&_.line]:before:[counter-increment:line] [&_.line]:before:content-[counter(line)] [&_.line]:before:inline-block [&_.line]:before:w-[2.5em] [&_.line]:before:pr-3 [&_.line]:before:text-right [&_.line]:before:text-text-tertiary [&_.line]:before:select-none"
    : "";
  const wrapClasses = wrap
    ? "[&_pre]:!whitespace-pre-wrap [&_pre]:break-words"
    : "";

  return (
    <div className="not-prose group/code relative">
      {html ? (
        <div
          className={cn(
            "[&_pre]:!m-0 [&_pre]:rounded-lg [&_pre]:!p-4 [&_pre]:!bg-[var(--surface-nav)] [&_pre]:overflow-auto [&_pre]:text-[0.8125rem]",
            lineNumberClasses,
            wrapClasses,
          )}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre
          className={cn(
            "!m-0 rounded-lg p-4 bg-surface-nav text-text-primary overflow-auto text-[0.8125rem]",
            wrap && "whitespace-pre-wrap break-words",
          )}
        >
          <code>{code}</code>
        </pre>
      )}
      <CopyButton code={code} />
    </div>
  );
}
