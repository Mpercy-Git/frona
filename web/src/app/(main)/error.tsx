"use client";

import { useEffect } from "react";
import { ArrowPathIcon } from "@heroicons/react/24/outline";

/**
 * Route-level backstop for the main app (chat, tasks, settings…). Without it, an
 * uncaught render error — most often while an agent is streaming — would blank
 * the page and force a full manual reload. Component-level boundaries handle the
 * common streaming surfaces; this catches everything else and offers an in-place
 * retry that re-renders the segment without losing the session.
 */
export default function MainError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error("[route-error:(main)]", error);
  }, [error]);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="text-sm text-text-secondary">Something went wrong displaying this view.</p>
      <div className="flex items-center gap-2">
        <button
          onClick={reset}
          className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-secondary px-3 py-1.5 text-sm text-text-primary hover:bg-surface-tertiary transition"
        >
          <ArrowPathIcon className="h-4 w-4" />
          Try again
        </button>
        <button
          onClick={() => window.location.reload()}
          className="rounded-lg px-3 py-1.5 text-sm text-text-tertiary hover:text-text-secondary transition"
        >
          Reload page
        </button>
      </div>
    </div>
  );
}
