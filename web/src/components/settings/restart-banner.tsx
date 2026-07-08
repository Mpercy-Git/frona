"use client";

import { api } from "@/lib/api-client";
import { useState } from "react";

interface RestartBannerProps {
  visible: boolean;
}

export function RestartBanner({ visible }: RestartBannerProps) {
  const [restarting, setRestarting] = useState(false);

  if (!visible) return null;

  const handleRestart = async () => {
    setRestarting(true);
    try {
      await api.post("/api/system/restart", {});
    } catch {}
    await new Promise((r) => setTimeout(r, 1000));
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      try {
        const res = await fetch("/", { cache: "no-store" });
        if (res.status < 500) {
          window.location.reload();
          return;
        }
      } catch {}
      await new Promise((r) => setTimeout(r, 2000));
    }
    window.location.reload();
  };

  return (
    <div className="rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 flex items-center justify-between gap-3">
      <p className="text-sm text-warning">
        Configuration saved. Restart the server for changes to take effect.
      </p>
      <button
        onClick={handleRestart}
        disabled={restarting}
        className="shrink-0 rounded-lg bg-warning px-3 py-1.5 text-xs font-medium text-surface hover:bg-warning/90 transition disabled:opacity-50"
      >
        {restarting ? "Restarting..." : "Restart Now"}
      </button>
    </div>
  );
}
