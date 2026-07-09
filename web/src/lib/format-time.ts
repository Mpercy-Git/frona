"use client";

import { useMemo } from "react";
import { useAuth } from "./auth";
import type { MessageResponse } from "./types";

const GAP_THRESHOLD_MS = 30 * 60 * 1000;

export function useTimezone(): string {
  const { user } = useAuth();
  return useMemo(
    () => user?.timezone || Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
    [user?.timezone],
  );
}

export function formatTime(iso: string, timeZone: string): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    timeZone,
  }).format(new Date(iso));
}

export function formatFullTimestamp(iso: string, timeZone: string): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone,
  }).format(new Date(iso));
}

function ymdInZone(d: Date, timeZone: string): string {
  // YYYY-MM-DD in the given zone — used to compare calendar days.
  return new Intl.DateTimeFormat("en-CA", {
    year: "numeric", month: "2-digit", day: "2-digit", timeZone,
  }).format(d);
}

export function formatDayLabel(iso: string, timeZone: string): string {
  const d = new Date(iso);
  const now = new Date();
  const today = ymdInZone(now, timeZone);
  const yesterday = ymdInZone(new Date(now.getTime() - 86400_000), timeZone);
  const target = ymdInZone(d, timeZone);

  if (target === today) return "Today";
  if (target === yesterday) return "Yesterday";

  const sameYear = target.slice(0, 4) === today.slice(0, 4);
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
    timeZone,
  }).format(d);
}

const THREE_DAYS_MS = 3 * 24 * 60 * 60 * 1000;

/**
 * Direction-neutral label anchored to "now" so it reads the same whether the
 * user is scrolling top→down or bottom→up. Switches to an absolute date+time
 * once the message is older than 3 days, because "72 hours ago" becomes
 * harder to map back to a real moment.
 */
export function formatGapLabel(iso: string, timeZone: string): string {
  const ageMs = Date.now() - new Date(iso).getTime();
  if (ageMs >= THREE_DAYS_MS) {
    const date = new Intl.DateTimeFormat("en-US", {
      month: "short", day: "numeric", timeZone,
    }).format(new Date(iso));
    const time = new Intl.DateTimeFormat("en-US", {
      hour: "numeric", minute: "2-digit", hour12: true, timeZone,
    }).format(new Date(iso));
    return `${date} at ${time}`.toUpperCase();
  }
  const mins = Math.max(1, Math.round(ageMs / 60_000));
  if (mins < 60) return `${mins} ${mins === 1 ? "minute" : "minutes"} ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours} ${hours === 1 ? "hour" : "hours"} ago`;
  const days = Math.round(hours / 24);
  return `${days} ${days === 1 ? "day" : "days"} ago`;
}

export interface TimeMarker {
  daySeparator?: string;
  gap?: string;
}

/**
 * Walks the message list in render order and decides where day separators
 * and gap markers belong. Gap markers are suppressed when a day separator
 * is already rendering, to avoid stacking two chips.
 */
export function computeTimeMarkers(
  messages: MessageResponse[],
  timeZone: string,
): Map<string, TimeMarker> {
  const out = new Map<string, TimeMarker>();
  let prev: MessageResponse | null = null;
  for (const msg of messages) {
    if (!msg.created_at) {
      prev = msg;
      continue;
    }
    const marker: TimeMarker = {};
    if (!prev) {
      // Skip a top-of-list "Today" — it's redundant; we know the user is
      // looking at today's chat. Other labels (Yesterday, May 18) still show.
      const label = formatDayLabel(msg.created_at, timeZone);
      if (label !== "Today") marker.daySeparator = label;
    } else if (prev.created_at) {
      const prevDay = ymdInZone(new Date(prev.created_at), timeZone);
      const currDay = ymdInZone(new Date(msg.created_at), timeZone);
      if (prevDay !== currDay) {
        marker.daySeparator = formatDayLabel(msg.created_at, timeZone);
      }
      const gapMs = new Date(msg.created_at).getTime() - new Date(prev.created_at).getTime();
      if (gapMs >= GAP_THRESHOLD_MS) {
        marker.gap = formatGapLabel(msg.created_at, timeZone);
      }
    }
    if (marker.daySeparator || marker.gap) {
      out.set(msg.id, marker);
    }
    prev = msg;
  }
  return out;
}
