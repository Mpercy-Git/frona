"use client";

import { useState } from "react";
import { ArrowDownTrayIcon } from "@heroicons/react/24/outline";
import type { MediaKind } from "@/lib/media-utils";

/** Same chip the non-previewable attachments use, so a codec the browser can't
 *  decode degrades to exactly what it rendered before. */
function DownloadChip({ url, filename }: { url: string; filename: string }) {
  return (
    <a
      href={url}
      download={filename}
      className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-tertiary px-3 py-2 text-xs text-text-secondary cursor-pointer hover:bg-surface-secondary transition-colors"
    >
      <span className="truncate max-w-[200px]">{filename}</span>
      <ArrowDownTrayIcon className="h-3.5 w-3.5 text-text-tertiary" />
    </a>
  );
}

/**
 * Inline `<audio>` / `<video>` player for a media file.
 *
 * `preload="metadata"` only yields a duration (and seekable timeline) because
 * `/api/files` answers `Range` requests with 206 — see the files route's
 * `range` module. Containers the browser can't decode fire `error` on the
 * element, which swaps in a download link.
 *
 * `variant` picks the sizing: `"inline"` fits a chat bubble, `"full"` fills a
 * preview dialog.
 */
export function MediaAttachment({
  url,
  filename,
  kind,
  variant = "inline",
}: {
  url: string;
  filename: string;
  kind: MediaKind;
  variant?: "inline" | "full";
}) {
  const [failed, setFailed] = useState(false);

  if (failed) return <DownloadChip url={url} filename={filename} />;

  if (kind === "audio") {
    return (
      <div
        className={`flex flex-col gap-1.5 rounded-lg border border-border bg-surface-tertiary px-3 py-2 ${
          variant === "full" ? "w-full" : "w-full max-w-xs"
        }`}
      >
        <span className="truncate text-xs text-text-secondary">{filename}</span>
        <audio
          controls
          preload="metadata"
          src={url}
          onError={() => setFailed(true)}
          className="w-full"
        />
      </div>
    );
  }

  return (
    <video
      controls
      preload="metadata"
      src={url}
      onError={() => setFailed(true)}
      className={
        variant === "full"
          ? "max-h-[70vh] w-full rounded-md bg-black"
          : "max-w-xs max-h-48 rounded-md border border-border bg-black"
      }
    />
  );
}
