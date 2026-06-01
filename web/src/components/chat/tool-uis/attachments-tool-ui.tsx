"use client";

import { useState } from "react";
import { makeAssistantToolUI, useMessage } from "@assistant-ui/react";
import { ArrowDownTrayIcon, XMarkIcon } from "@heroicons/react/24/outline";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { CodeBlock } from "@/components/ui/code-block";
import type { Attachment } from "@/lib/types";

function FilePreviewContent({ content, attachment }: { content: string; attachment: Attachment }) {
  const lang = languageFromFilename(attachment.filename);

  if (attachment.content_type === "text/markdown" || attachment.filename.endsWith(".md") || attachment.filename.endsWith(".mdx")) {
    return (
      <div className="prose prose-sm max-w-none text-text-primary prose-headings:text-text-primary prose-strong:text-text-primary prose-a:text-accent prose-code:text-text-primary prose-code:before:content-none prose-code:after:content-none prose-blockquote:text-text-secondary prose-blockquote:border-border">
        <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
          code({ className, children, ...props }) {
            const match = /language-(\w+)/.exec(className || "");
            const code = String(children).replace(/\n$/, "");
            if (match) return <CodeBlock code={code} language={match[1]} />;
            return <code className={className} {...props}>{children}</code>;
          },
        }}>
          {content}
        </ReactMarkdown>
      </div>
    );
  }

  if (lang) {
    return (
      <div className="[&_pre]:!rounded-none [&_pre]:!m-0 min-h-full">
        <CodeBlock code={content} language={lang} />
      </div>
    );
  }

  return <pre className="whitespace-pre-wrap text-sm text-text-primary font-mono">{content}</pre>;
}

function FilePreviewModal({ attachment, onClose }: { attachment: Attachment; onClose: () => void }) {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const isMarkdown = attachment.content_type === "text/markdown" || attachment.filename.endsWith(".md") || attachment.filename.endsWith(".mdx");
  const isCode = !isMarkdown && !!languageFromFilename(attachment.filename);

  useState(() => {
    if (!attachment.url) return;
    fetch(attachment.url)
      .then((r) => r.text())
      .then((text) => { setContent(text); setLoading(false); })
      .catch(() => { setContent("Failed to load file."); setLoading(false); });
  });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div
        className="relative flex flex-col w-[90vw] max-w-3xl max-h-[80vh] rounded-xl border border-border bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <span className="text-sm font-medium text-text-primary truncate">{attachment.filename}</span>
          <div className="flex items-center gap-2">
            <a
              href={attachment.url}
              download={attachment.filename}
              className="flex items-center gap-1.5 rounded-md bg-surface-tertiary px-2.5 py-1.5 text-xs text-text-secondary hover:text-text-primary hover:bg-surface-secondary transition-colors"
            >
              <ArrowDownTrayIcon className="h-3.5 w-3.5" />
              Download
            </a>
            <button onClick={onClose} className="text-text-tertiary hover:text-text-primary transition-colors">
              <XMarkIcon className="h-5 w-5" />
            </button>
          </div>
        </div>
        <div className={`flex-1 overflow-auto ${isCode ? "" : "p-4"}`}>
          {loading ? (
            <p className="text-sm text-text-tertiary p-4">Loading...</p>
          ) : <FilePreviewContent content={content ?? ""} attachment={attachment} />}
        </div>
      </div>
    </div>
  );
}

const EXT_TO_LANGUAGE: Record<string, string> = {
  js: "javascript", jsx: "jsx", ts: "typescript", tsx: "tsx",
  py: "python", rb: "ruby", rs: "rust", go: "go", java: "java",
  c: "c", cpp: "cpp", h: "c", hpp: "cpp", cs: "csharp",
  swift: "swift", kt: "kotlin", scala: "scala", php: "php",
  sh: "bash", bash: "bash", zsh: "bash", fish: "fish",
  html: "html", css: "css", scss: "scss", less: "less",
  json: "json", yaml: "yaml", yml: "yaml", toml: "toml", xml: "xml",
  sql: "sql", graphql: "graphql", gql: "graphql",
  md: "markdown", mdx: "mdx", tex: "latex",
  dockerfile: "dockerfile", makefile: "makefile",
  tf: "hcl", hcl: "hcl", nix: "nix",
  lua: "lua", r: "r", dart: "dart", zig: "zig", v: "v",
  svelte: "svelte", vue: "vue", astro: "astro",
};

function languageFromFilename(filename: string): string | null {
  const ext = filename.split(".").pop()?.toLowerCase();
  if (!ext) return null;
  return EXT_TO_LANGUAGE[ext] ?? null;
}

function canPreviewFile(contentType: string, filename: string): boolean {
  if (contentType.startsWith("text/")) return true;
  if (contentType === "application/json") return true;
  if (contentType === "application/xml") return true;
  if (languageFromFilename(filename)) return true;
  return false;
}

function AttachmentItem({ attachment }: { attachment: Attachment }) {
  const url = attachment.url;
  const isImage = attachment.content_type.startsWith("image/");
  const canPreview = canPreviewFile(attachment.content_type, attachment.filename);
  const [showPreview, setShowPreview] = useState(false);

  if (!url) return null;

  if (isImage) {
    return (
      <a href={url} target="_blank" rel="noopener noreferrer">
        <img src={url} alt={attachment.filename} className="max-w-xs max-h-48 rounded-md border border-border" />
      </a>
    );
  }

  if (canPreview) {
    return (
      <>
        <button
          onClick={() => setShowPreview(true)}
          className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-tertiary px-3 py-2 text-xs text-text-secondary cursor-pointer hover:bg-surface-secondary transition-colors"
        >
          <span className="truncate max-w-[200px]">{attachment.filename}</span>
        </button>
        {showPreview && <FilePreviewModal attachment={attachment} onClose={() => setShowPreview(false)} />}
      </>
    );
  }

  return (
    <a
      href={url}
      download={attachment.filename}
      className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-tertiary px-3 py-2 text-xs text-text-secondary cursor-pointer hover:bg-surface-secondary transition-colors"
    >
      <span className="truncate max-w-[200px]">{attachment.filename}</span>
      <ArrowDownTrayIcon className="h-3.5 w-3.5 text-text-tertiary" />
    </a>
  );
}

function AttachmentsRender() {
  const message = useMessage();
  const attachments = (message.metadata as Record<string, any>)?.custom?.attachments as Attachment[] | undefined;

  if (!attachments?.length) return null;

  return (
    <div className="flex flex-wrap gap-2 mt-2">
      {attachments.map((att, i) => (
        <AttachmentItem key={i} attachment={att} />
      ))}
    </div>
  );
}

export const AttachmentsToolUI = makeAssistantToolUI<Record<string, never>, string>({
  toolName: "Attachments",
  render: AttachmentsRender,
});
