"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { PreviewPage } from "@/components/preview/preview-page";
import type { PreviewSource } from "@/components/preview/preview-page";

/** Static page — Next.js `output: "export"` rules out runtime-dynamic
 *  routes — so the share id or file path arrives via `?id=` or `?path=`.
 *  The backend's `/p/{slug}` route 303-redirects clean URLs into this shape.
 *  `useSearchParams` needs a `Suspense` parent. */
function Inner() {
  const params = useSearchParams();
  const id = params.get("id");
  const path = params.get("path");

  let source: PreviewSource | null = null;
  if (id) {
    source = { kind: "short", id };
  } else if (path) {
    // The channel adapter's URL-encoded `?path=...` round-trips through the
    // backend redirector; decode before splitting on `/`.
    const decoded = decodeURIComponent(path).replace(/^\/+/, "");
    const segs = decoded.split("/");
    if (segs.length >= 3) {
      source = {
        kind: "long",
        owner: segs[0],
        handle: segs[1],
        path: segs.slice(2).join("/"),
      };
    }
  }

  if (!source) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-surface">
        <div className="max-w-md text-center px-6">
          <h1 className="text-lg font-medium text-text-primary">Invalid preview link</h1>
          <p className="mt-2 text-sm text-text-secondary">
            The URL is missing the share id or file path.
          </p>
        </div>
      </div>
    );
  }

  return <PreviewPage source={source} />;
}

export default function Page() {
  return (
    <Suspense fallback={<div className="min-h-screen flex items-center justify-center bg-surface text-text-tertiary text-sm">Loading…</div>}>
      <Inner />
    </Suspense>
  );
}
