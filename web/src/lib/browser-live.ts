import { API_URL } from "./api-client";

/// Messages the live-view socket sends. `width`/`height` are the page's
/// viewport in CSS pixels, which is the space input coordinates are sent in.
export type LiveServerMessage =
  | { type: "status"; state: "connecting" | "live" | "ended"; message?: string }
  | { type: "frame"; data: string; width: number; height: number }
  | { type: "tabs"; tabs: LiveTab[]; current: string }
  | { type: "error"; message: string };

export interface LiveTab {
  id: string;
  url: string;
  title: string;
}

/// CDP's modifier bit field: Alt=1, Ctrl=2, Meta/Command=4, Shift=8.
export function modifierBits(e: {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): number {
  return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

/// `MouseEvent.button` → CDP's button name.
export function mouseButtonName(button: number): string {
  return ["left", "middle", "right", "back", "forward"][button] ?? "none";
}

/// Map a point on the displayed frame to page CSS pixels. The frame is drawn
/// with `object-fit: contain`, so there may be letterboxing on one axis.
/// Returns null for a point in the letterbox, outside the page.
export function toPagePoint(
  clientX: number,
  clientY: number,
  box: { left: number; top: number; width: number; height: number },
  page: { width: number; height: number },
): { x: number; y: number } | null {
  if (!page.width || !page.height || !box.width || !box.height) return null;
  const scale = Math.min(box.width / page.width, box.height / page.height);
  const drawnW = page.width * scale;
  const drawnH = page.height * scale;
  const offsetX = box.left + (box.width - drawnW) / 2;
  const offsetY = box.top + (box.height - drawnH) / 2;
  const x = (clientX - offsetX) / scale;
  const y = (clientY - offsetY) / scale;
  if (x < 0 || y < 0 || x > page.width || y > page.height) return null;
  return { x, y };
}

/// The live-view socket URL, on whichever origin the API is served from.
/// Behind a TLS-terminating proxy (Caddy and the like) the page is https, so
/// this comes out as wss on the same host — nothing extra to route.
export function liveSocketUrl(profile: string, token: string, origin = window.location.origin): string {
  const base = new URL(API_URL || origin);
  base.protocol = base.protocol === "https:" ? "wss:" : "ws:";
  base.pathname = `${base.pathname.replace(/\/$/, "")}/api/browser/live/ws`;
  base.search = new URLSearchParams({ profile, token }).toString();
  return base.toString();
}

/// Where a takeover's "Open live browser" button goes, as an in-app path.
/// Takeovers store an absolute link built from the server's public URL, which
/// may not be the origin this tab is on (e.g. a LAN address vs. the public
/// domain), so only the path and query are kept. Older takeovers pointed at the
/// removed Browserless debugger proxy; those open the live view of the user's
/// current profile.
export function takeoverHref(debuggerUrl: string): string {
  let url: URL;
  try {
    url = new URL(debuggerUrl, "http://placeholder");
  } catch {
    return "/browser";
  }
  if (url.pathname !== "/browser") return "/browser";
  const profile = url.searchParams.get("profile");
  return profile ? `/browser?${new URLSearchParams({ profile })}` : "/browser";
}
