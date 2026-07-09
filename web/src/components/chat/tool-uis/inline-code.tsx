"use client";

import { useState, useEffect } from "react";
import { codeToHtml } from "shiki";
import { CODE_THEME } from "@/lib/theme";

export function InlineCode({ code, language }: { code: string; language?: string }) {
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
        if (!cancelled) setHtml(null);
      });
    return () => { cancelled = true; };
  }, [code, language]);

  if (html) {
    return (
      <div
        className="[&_pre]:!m-0 [&_pre]:!p-0 [&_pre]:!bg-transparent [&_pre]:text-xs [&_pre]:overflow-auto"
        dangerouslySetInnerHTML={{ __html: html }}
      />
    );
  }

  return (
    <pre className="!m-0 !p-0 !bg-transparent text-xs overflow-auto">
      <code>{code}</code>
    </pre>
  );
}
