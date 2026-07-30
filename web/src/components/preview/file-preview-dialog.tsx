"use client";

import { useCallback, useEffect, useState } from "react";
import {
  ArrowDownTrayIcon,
  ArrowTopRightOnSquareIcon,
  XMarkIcon,
} from "@heroicons/react/24/outline";
import { presignFile } from "@/lib/api-client";
import { previewKind } from "@/lib/preview-kind";
import { FilePreviewContent } from "@/components/preview/file-preview-content";
import { MediaAttachment } from "@/components/preview/media-attachment";

/** A log or dataset can be far larger than anything worth rendering, so the
 *  text preview asks for a prefix and says so when it truncates. The backing
 *  `Range` support is what keeps this to one short request. */
const TEXT_PREVIEW_BYTES = 256 * 1024;

export type PreviewTarget = {
  filename: string;
  owner: string;
  path: string;
};

type TextState =
  | { status: "loading" }
  | { status: "ok"; content: string; truncated: boolean }
  | { status: "error" };

/** Total size out of `Content-Range: bytes 0-1023/58234`, if the server sent one. */
function totalFromContentRange(header: string | null): number | null {
  const total = header?.split("/")[1];
  const parsed = Number(total);
  return total && Number.isFinite(parsed) ? parsed : null;
}

function TextPreview({ url, target }: { url: string; target: PreviewTarget }) {
  const [state, setState] = useState<TextState>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        // A Range header makes this a preflighted request cross-origin. If the
        // server's CORS config predates that allowance, fall back to a plain
        // fetch and trim client-side rather than failing the preview.
        const res = await fetch(url, {
          headers: { Range: `bytes=0-${TEXT_PREVIEW_BYTES - 1}` },
        }).catch(() => fetch(url));
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const text = await res.text();
        if (cancelled) return;

        // 206 means we got the prefix we asked for; anything else means the
        // server handed over the whole file and we trim it ourselves.
        const total = totalFromContentRange(res.headers.get("content-range"));
        const truncated =
          res.status === 206
            ? total === null || total > TEXT_PREVIEW_BYTES
            : text.length > TEXT_PREVIEW_BYTES;

        setState({
          status: "ok",
          content: truncated ? text.slice(0, TEXT_PREVIEW_BYTES) : text,
          truncated,
        });
      } catch {
        if (!cancelled) setState({ status: "error" });
      }
    })();

    return () => { cancelled = true; };
  }, [url]);

  if (state.status === "loading") {
    return <p className="p-4 text-sm text-text-tertiary">Loading…</p>;
  }
  if (state.status === "error") {
    return <p className="p-4 text-sm text-text-tertiary">Couldn&apos;t load this file.</p>;
  }

  return (
    <>
      {state.truncated && (
        <p className="border-b border-border bg-surface-secondary px-4 py-2 text-xs text-text-tertiary">
          Showing the first {TEXT_PREVIEW_BYTES / 1024} KB — download the file for the rest.
        </p>
      )}
      <FilePreviewContent
        content={state.content}
        filename={target.filename}
        contentType=""
      />
    </>
  );
}

function UnsupportedPreview({ url, filename }: { url: string; filename: string }) {
  return (
    <div className="flex flex-col items-center gap-3 p-10 text-center">
      <p className="text-sm text-text-secondary">
        No preview for this file type.
      </p>
      <div className="flex items-center gap-2">
        <a
          href={url}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1.5 rounded-md bg-surface-tertiary px-3 py-2 text-xs text-text-secondary hover:bg-surface-secondary hover:text-text-primary transition-colors"
        >
          <ArrowTopRightOnSquareIcon className="h-3.5 w-3.5" />
          Open in new tab
        </a>
        <a
          href={url}
          download={filename}
          className="inline-flex items-center gap-1.5 rounded-md bg-surface-tertiary px-3 py-2 text-xs text-text-secondary hover:bg-surface-secondary hover:text-text-primary transition-colors"
        >
          <ArrowDownTrayIcon className="h-3.5 w-3.5" />
          Download
        </a>
      </div>
    </div>
  );
}

function PreviewBody({ url, target }: { url: string; target: PreviewTarget }) {
  const kind = previewKind(undefined, target.filename);

  switch (kind) {
    case "image":
      return (
        <div className="flex items-center justify-center p-4">
          <img
            src={url}
            alt={target.filename}
            className="max-h-[70vh] max-w-full object-contain"
          />
        </div>
      );
    case "audio":
    case "video":
      return (
        <div className="p-4">
          <MediaAttachment
            url={url}
            filename={target.filename}
            kind={kind}
            variant="full"
          />
        </div>
      );
    case "pdf":
      return (
        <iframe src={url} title={target.filename} className="h-[70vh] w-full" />
      );
    case "text":
      return <TextPreview url={url} target={target} />;
    default:
      return <UnsupportedPreview url={url} filename={target.filename} />;
  }
}

/**
 * Modal preview for a file in the Files tab. Everything it renders comes from
 * one presigned URL, so the dialog works the same for user files and agent
 * workspace files.
 */
export function FilePreviewDialog({
  target,
  onClose,
}: {
  target: PreviewTarget;
  onClose: () => void;
}) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    presignFile(target.owner, target.path)
      .then((u) => { if (!cancelled) setUrl(u); })
      .catch(() => { if (!cancelled) setFailed(true); });
    return () => { cancelled = true; };
  }, [target.owner, target.path]);

  const onKeyDown = useCallback(
    (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); },
    [onClose],
  );

  useEffect(() => {
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onKeyDown]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[85vh] w-[90vw] max-w-4xl flex-col rounded-xl border border-border bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-4 border-b border-border px-4 py-3">
          <span className="truncate text-sm font-medium text-text-primary">
            {target.filename}
          </span>
          <div className="flex shrink-0 items-center gap-2">
            {url && (
              <>
                <a
                  href={url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center gap-1.5 rounded-md bg-surface-tertiary px-2.5 py-1.5 text-xs text-text-secondary hover:bg-surface-secondary hover:text-text-primary transition-colors"
                >
                  <ArrowTopRightOnSquareIcon className="h-3.5 w-3.5" />
                  Open
                </a>
                <a
                  href={url}
                  download={target.filename}
                  className="flex items-center gap-1.5 rounded-md bg-surface-tertiary px-2.5 py-1.5 text-xs text-text-secondary hover:bg-surface-secondary hover:text-text-primary transition-colors"
                >
                  <ArrowDownTrayIcon className="h-3.5 w-3.5" />
                  Download
                </a>
              </>
            )}
            <button
              onClick={onClose}
              aria-label="Close preview"
              className="text-text-tertiary hover:text-text-primary transition-colors"
            >
              <XMarkIcon className="h-5 w-5" />
            </button>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-auto">
          {failed ? (
            <p className="p-4 text-sm text-text-tertiary">Couldn&apos;t open this file.</p>
          ) : url ? (
            <PreviewBody url={url} target={target} />
          ) : (
            <p className="p-4 text-sm text-text-tertiary">Loading…</p>
          )}
        </div>
      </div>
    </div>
  );
}
