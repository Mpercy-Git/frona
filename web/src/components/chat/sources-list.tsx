"use client";

import { useState } from "react";
import { ChevronDownIcon } from "@heroicons/react/24/outline";
import type { Citation } from "@/lib/types";

function hostname(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

export function SourcesList({ citations }: { citations: Citation[] }) {
  const [open, setOpen] = useState(false);

  if (!citations.length) return null;

  return (
    <div className="mt-1.5 w-full">
      <button
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex items-center gap-1 text-xs text-text-tertiary hover:text-text-primary transition-colors cursor-pointer"
      >
        <ChevronDownIcon className={`h-3 w-3 transition-transform ${open ? "rotate-180" : ""}`} />
        Sources ({citations.length})
      </button>
      {open && (
        <ul className="mt-1.5 flex flex-col gap-1 pl-4">
          {citations.map((c, i) => (
            <li key={i} className="truncate text-xs">
              <a
                href={c.url}
                target="_blank"
                rel="noopener noreferrer"
                title={c.url}
                className="text-text-secondary hover:text-accent transition-colors"
              >
                {c.title || hostname(c.url)}
              </a>
              {c.title && <span className="text-text-tertiary"> — {hostname(c.url)}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
