"use client";

import { LockClosedIcon } from "@heroicons/react/24/outline";
import { cn } from "@/lib/utils";
import { groupEvents, type SandboxEvent } from "./shell-sandbox";

const SEVERITY_CLASS: Record<SandboxEvent["severity"], string> = {
  low: "border-border bg-surface-tertiary/50 text-text-secondary",
  high: "border-warning/30 bg-warning/10 text-text-primary",
  critical: "border-danger/30 bg-danger/10 text-text-primary",
};

const ICON_CLASS: Record<SandboxEvent["severity"], string> = {
  low: "text-text-tertiary",
  high: "text-warning",
  critical: "text-danger",
};

export function SandboxBlock({ events }: { events: SandboxEvent[] }) {
  if (events.length === 0) return null;

  const groups = Array.from(groupEvents(events).entries());

  return (
    <div className="flex flex-col gap-2 px-3 pb-3">
      {groups.map(([key, group]) => {
        const { cap, act, severity } = group[0];
        const tip = group.find((e) => e.tip)?.tip;
        return (
          <div
            key={key}
            className={cn(
              "rounded-md border p-2 text-xs",
              SEVERITY_CLASS[severity],
            )}
          >
            <div className="flex items-center gap-1.5 font-semibold">
              <LockClosedIcon className={cn("h-3.5 w-3.5", ICON_CLASS[severity])} />
              <span>
                Sandbox {act}: {group.length} blocked{" "}
                <span className="font-mono font-normal text-text-tertiary">
                  ({cap})
                </span>
              </span>
            </div>
            <ul className="mt-1 ml-5 list-disc font-mono text-text-secondary">
              {group.map((ev, i) => (
                <li key={`${ev.target}-${i}`}>{ev.target || "—"}</li>
              ))}
            </ul>
            {tip && (
              <p className="mt-2 text-text-tertiary">
                Tip:{" "}
                <code className="font-mono text-text-secondary">{tip}</code>
              </p>
            )}
          </div>
        );
      })}
    </div>
  );
}
