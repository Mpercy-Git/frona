"use client";

import type { ChatActivity } from "@/lib/chat-activity-context";

/// The halo that says "the agent has a live turn here". Two counter-weighted
/// conic gradients masked into a ring, which reads as motion at 32px next to
/// an avatar and still reads at 10px in a list row.
///
/// `halo` sits the ring *outside* the box so it can wrap content (the agent
/// avatar on the last message); without it the ring fills the box and is used
/// on its own as a list marker.
export function WorkingRing({
  size = 32,
  halo = false,
  label,
  children,
}: {
  size?: number;
  halo?: boolean;
  /// Omit where the ring only repeats what the surrounding content already
  /// says (the avatar on a streaming message) — it is then hidden from
  /// assistive tech rather than announced twice.
  label?: string;
  children?: React.ReactNode;
}) {
  const inset = halo ? "-3px" : "0px";
  const maskWidth = halo ? 2 : Math.max(1.5, size / 8);
  const mask = `radial-gradient(farthest-side, transparent calc(100% - ${maskWidth}px), #fff calc(100% - ${maskWidth}px))`;
  // As a bare marker this sits inside buttons and other phrasing content, so
  // it has to be a span there; wrapping an avatar it is flow content already.
  const Tag = halo ? "div" : "span";

  return (
    <Tag
      className={`relative shrink-0 ${halo ? "" : "inline-flex"}`}
      style={{ height: size, width: size }}
      // `img` rather than `status`: a list can hold many of these, and a live
      // region per row would announce a burst of updates every time an agent
      // picked up work. The aggregate dot is the one that speaks.
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      title={label}
    >
      <span
        className="working-ring absolute rounded-full"
        style={{
          inset,
          background: "conic-gradient(from 0deg, transparent 0%, var(--accent) 30%, transparent 60%)",
          mask,
          WebkitMask: mask,
        }}
      />
      <span
        className="working-ring working-ring-trail absolute rounded-full"
        style={{
          inset,
          background: "conic-gradient(from 180deg, transparent 0%, var(--accent) 20%, transparent 50%)",
          mask,
          WebkitMask: mask,
        }}
      />
      {children}
    </Tag>
  );
}

/// The counterpart to the ring: the turn is parked and *you* are what it is
/// waiting for. Deliberately static — motion would read as "still working",
/// which is the opposite of what this means — and in the warning colour so the
/// two are never confused at a glance.
export function WaitingBadge({
  size = 10,
  label,
}: {
  size?: number;
  label?: string;
}) {
  return (
    <span
      className="relative flex shrink-0 items-center justify-center"
      style={{ height: size, width: size }}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      title={label}
    >
      <span className="absolute inset-0 rounded-full bg-warning/25" />
      <span
        className="rounded-full bg-warning"
        style={{ height: Math.max(4, size / 2), width: Math.max(4, size / 2) }}
      />
    </span>
  );
}

/// Row-sized marker for one chat. Renders nothing when the chat is idle, so
/// callers can drop it straight into a list row.
export function ChatActivityIndicator({
  activity,
  size = 10,
  workingLabel,
  waitingLabel,
}: {
  activity: ChatActivity;
  size?: number;
  workingLabel?: string;
  waitingLabel?: string;
}) {
  if (activity === "working") return <WorkingRing size={size} label={workingLabel} />;
  if (activity === "waiting") return <WaitingBadge size={size} label={waitingLabel} />;
  return null;
}

/// The same information compressed to a single dot, for surfaces that stand in
/// for the whole list when it is out of view — the collapsed navigation rail
/// and the mobile drawer button.
export function ActivityDot({
  activity,
  label,
}: {
  activity: ChatActivity;
  label: string;
}) {
  if (activity === "idle") return null;
  return (
    <span
      role="status"
      aria-label={label}
      title={label}
      className={`h-2 w-2 rounded-full ${
        activity === "working" ? "bg-accent activity-dot-pulse" : "bg-warning"
      }`}
    />
  );
}
