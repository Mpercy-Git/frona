"use client";

import { useState, useRef, useEffect, useCallback } from "react";
import {
  Squares2X2Icon,
  TrashIcon,
  StopIcon,
  PlayIcon,
  XMarkIcon,
} from "@heroicons/react/24/outline";
import { api, API_URL } from "@/lib/api-client";
import { useMobile } from "@/lib/use-mobile";
import type { AppResponse } from "@/lib/types";

function statusDot(status: AppResponse["status"]): string {
  switch (status) {
    case "running":
    case "serving":
      return "bg-green-400";
    case "starting":
      return "bg-yellow-400";
    case "stopped":
    case "hibernated":
      return "bg-gray-400";
    case "failed":
      return "bg-red-400";
    default:
      return "bg-gray-400";
  }
}

function AppList({ apps, onAction, onClose }: {
  apps: AppResponse[];
  onAction: (action: "stop" | "restart" | "delete", app: AppResponse) => void;
  onClose: () => void;
}) {
  return (
    <>
      <div className="flex items-center justify-between px-4 py-2 border-b border-border shrink-0">
        <span className="text-sm font-medium text-text-secondary">Apps</span>
        <button
          onClick={onClose}
          className="flex items-center justify-center h-8 w-8 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition md:hidden"
        >
          <XMarkIcon className="h-5 w-5" />
        </button>
      </div>
      <div className="flex-1 overflow-y-auto">
        {apps.map((app) => (
          <div
            key={app.id}
            className="group flex items-center gap-2 px-4 py-3 md:py-2 hover:bg-surface-tertiary transition cursor-pointer"
            onClick={() => {
              window.open(`${API_URL}/apps/${app.handle}/`, "_blank");
              onClose();
            }}
          >
            <span className={`h-2 w-2 shrink-0 rounded-full ${statusDot(app.status)}`} />
            <span className="flex-1 text-sm text-text-secondary group-hover:text-text-primary">
              {app.name}
            </span>
            {app.status === "running" ? (
              <button
                onClick={(e) => { e.stopPropagation(); onAction("stop", app); }}
                className="p-1 rounded text-text-tertiary hover:text-text-primary transition opacity-0 group-hover:opacity-100"
                title="Stop"
              >
                <StopIcon className="h-4 w-4" />
              </button>
            ) : (app.status === "stopped" || app.status === "failed" || app.status === "hibernated") && app.kind !== "static" ? (
              <button
                onClick={(e) => { e.stopPropagation(); onAction("restart", app); }}
                className="p-1 rounded text-text-tertiary hover:text-text-primary transition opacity-0 group-hover:opacity-100"
                title="Start"
              >
                <PlayIcon className="h-4 w-4" />
              </button>
            ) : null}
            <button
              onClick={(e) => { e.stopPropagation(); onAction("delete", app); }}
              className="p-1 rounded text-text-tertiary hover:text-error-text transition opacity-0 group-hover:opacity-100"
              title="Delete"
            >
              <TrashIcon className="h-4 w-4" />
            </button>
          </div>
        ))}
        {apps.length === 0 && (
          <p className="px-4 py-4 text-center text-xs text-text-tertiary">
            No apps deployed
          </p>
        )}
      </div>
    </>
  );
}

export function AppDropdown() {
  const [open, setOpen] = useState(false);
  const [apps, setApps] = useState<AppResponse[]>([]);
  const ref = useRef<HTMLDivElement>(null);
  const mobile = useMobile();

  const fetchApps = useCallback(async () => {
    try {
      const data = await api.get<AppResponse[]>("/api/apps");
      setApps(data);
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    fetchApps();
  }, [fetchApps]);

  useEffect(() => {
    if (open) fetchApps();
  }, [open, fetchApps]);

  useEffect(() => {
    if (!open || mobile) return;
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open, mobile]);

  const handleAction = async (action: "stop" | "restart" | "delete", app: AppResponse) => {
    if (action === "delete" && !confirm(`Delete app "${app.name}"?`)) return;
    try {
      if (action === "stop") await api.post(`/api/apps/${app.handle}/stop`, {});
      else if (action === "restart") await api.post(`/api/apps/${app.handle}/restart`, {});
      else if (action === "delete") await api.delete(`/api/apps/${app.handle}`);
    } catch (err) {
      alert(`Failed to ${action} "${app.name}": ${err instanceof Error ? err.message : "Unknown error"}`);
    }
    fetchApps();
  };

  if (mobile) {
    return (
      <>
        <button
          onClick={() => setOpen(true)}
          className="relative flex items-center justify-center h-10 w-10 rounded-full bg-surface-tertiary text-text-secondary hover:brightness-125 transition cursor-pointer"
          title="Apps"
        >
          <Squares2X2Icon className="h-5 w-5" />
        </button>
        {open && (
          <>
            <div className="fixed inset-0 z-[69] bg-black/40" onClick={() => setOpen(false)} />
            <div className="fixed inset-y-0 right-0 z-[70] w-[85vw] max-w-sm bg-surface shadow-xl flex flex-col transition-transform duration-200 ease-out">
              <AppList apps={apps} onAction={handleAction} onClose={() => setOpen(false)} />
            </div>
          </>
        )}
      </>
    );
  }

  return (
    <div ref={ref} className="relative flex items-center">
      <button
        onClick={() => setOpen((v) => !v)}
        className={`relative flex items-center justify-center h-10 w-10 transition cursor-pointer ${
          open ? "rounded-t-xl rounded-b-none bg-surface-secondary text-text-primary z-[61] border border-border border-b-0" : "rounded-full bg-surface-tertiary text-text-secondary hover:brightness-125"
        }`}
        title="Apps"
      >
        <Squares2X2Icon className="h-5 w-5" />
      </button>

      {open && (
        <div className="absolute right-0 top-full z-[60] w-64 rounded-xl rounded-tr-none border border-border bg-surface-secondary shadow-lg">
          <div className="absolute -top-px right-0 w-[calc(theme(spacing.10)-3px)] h-[2px] bg-surface-secondary z-[60]" />
          <div className="pb-1">
            <AppList apps={apps} onAction={handleAction} onClose={() => setOpen(false)} />
          </div>
        </div>
      )}
    </div>
  );
}
