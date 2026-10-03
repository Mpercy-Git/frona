"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeftIcon,
  ArrowPathIcon,
  ArrowRightIcon,
} from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import {
  liveSocketUrl,
  modifierBits,
  mouseButtonName,
  toPagePoint,
  type LiveServerMessage,
  type LiveTab,
} from "@/lib/browser-live";

type Status = "connecting" | "live" | "ended";

/// Two presses this close in time and space count as a double click, which
/// pointer events (unlike mouse events) don't report via `detail`.
const MULTI_CLICK_MS = 400;
const MULTI_CLICK_PX = 5;

/// Interactive live view of the agent's browser. Frames stream in over a
/// WebSocket on the app's own origin, and the person's mouse and keyboard are
/// replayed into the same browser session the agent is driving.
export function BrowserLiveView({ profile }: { profile: string | null }) {
  const socketRef = useRef<WebSocket | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const keyboardRef = useRef<HTMLTextAreaElement | null>(null);
  const pageSize = useRef({ width: 0, height: 0 });
  const pendingMove = useRef<{ x: number; y: number; buttons: number; modifiers: number } | null>(null);
  const lastPress = useRef({ time: 0, x: 0, y: 0, count: 0 });
  // Keys whose press was forwarded. A release is only sent for these, so a key
  // pressed elsewhere (Enter in the address bar) doesn't arrive half-done.
  const keysDown = useRef(new Set<string>());

  const [status, setStatus] = useState<Status>("connecting");
  const [endMessage, setEndMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hasFrame, setHasFrame] = useState(false);
  const [tabs, setTabs] = useState<LiveTab[]>([]);
  const [currentTab, setCurrentTab] = useState<string | null>(null);
  const [address, setAddress] = useState("");
  const [editingAddress, setEditingAddress] = useState(false);
  const [attempt, setAttempt] = useState(0);

  const send = useCallback((message: Record<string, unknown>) => {
    const socket = socketRef.current;
    if (socket && socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
  }, []);

  useEffect(() => {
    let cancelled = false;
    setStatus("connecting");
    setEndMessage(null);
    setError(null);

    (async () => {
      let link: { profile: string; token: string };
      try {
        const query = profile ? `?${new URLSearchParams({ profile })}` : "";
        link = await api.get<{ profile: string; token: string }>(`/api/browser/live/link${query}`);
      } catch (e) {
        if (!cancelled) {
          setStatus("ended");
          setEndMessage(e instanceof Error ? e.message : "Could not open the live view.");
        }
        return;
      }
      if (cancelled) return;

      const socket = new WebSocket(liveSocketUrl(link.profile, link.token));
      socketRef.current = socket;

      socket.onmessage = (event) => {
        let msg: LiveServerMessage;
        try {
          msg = JSON.parse(event.data as string);
        } catch {
          return;
        }
        switch (msg.type) {
          case "frame":
            // Straight onto the element: a React render per frame is wasted work.
            pageSize.current = { width: msg.width, height: msg.height };
            if (imgRef.current) imgRef.current.src = `data:image/jpeg;base64,${msg.data}`;
            setHasFrame(true);
            break;
          case "status":
            setStatus(msg.state);
            if (msg.state === "ended") setEndMessage(msg.message ?? "The live view ended.");
            break;
          case "tabs":
            setTabs(msg.tabs);
            setCurrentTab(msg.current);
            break;
          case "error":
            setError(msg.message);
            break;
        }
      };
      socket.onclose = () => {
        if (socketRef.current === socket) socketRef.current = null;
        if (!cancelled) {
          setStatus("ended");
          setEndMessage((prev) => prev ?? "Disconnected from the browser.");
        }
      };
    })();

    return () => {
      cancelled = true;
      socketRef.current?.close();
      socketRef.current = null;
    };
  }, [profile, attempt]);

  // Keep the address bar in step with the page, except while it's being typed in.
  const current = tabs.find((t) => t.id === currentTab);
  useEffect(() => {
    if (!editingAddress) setAddress(current?.url ?? "");
  }, [current?.url, editingAddress]);

  // Errors are momentary hints, not state.
  useEffect(() => {
    if (!error) return;
    const timer = window.setTimeout(() => setError(null), 4000);
    return () => window.clearTimeout(timer);
  }, [error]);

  const pagePoint = useCallback((clientX: number, clientY: number) => {
    const img = imgRef.current;
    if (!img) return null;
    return toPagePoint(clientX, clientY, img.getBoundingClientRect(), pageSize.current);
  }, []);

  const onPointerDown = (e: React.PointerEvent<HTMLImageElement>) => {
    const point = pagePoint(e.clientX, e.clientY);
    if (!point) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    // Mouse users type straight away; on touch, focusing would pop the
    // on-screen keyboard on every tap, so that's left to the keyboard button.
    if (e.pointerType === "mouse") keyboardRef.current?.focus({ preventScroll: true });

    const last = lastPress.current;
    const repeat =
      e.timeStamp - last.time < MULTI_CLICK_MS &&
      Math.abs(e.clientX - last.x) < MULTI_CLICK_PX &&
      Math.abs(e.clientY - last.y) < MULTI_CLICK_PX;
    const count = repeat ? last.count + 1 : 1;
    lastPress.current = { time: e.timeStamp, x: e.clientX, y: e.clientY, count };

    send({
      type: "mouse",
      action: "pressed",
      ...point,
      button: mouseButtonName(e.button),
      buttons: e.buttons,
      click_count: count,
      modifiers: modifierBits(e),
    });
  };

  const onPointerUp = (e: React.PointerEvent<HTMLImageElement>) => {
    const point = pagePoint(e.clientX, e.clientY);
    if (!point) return;
    send({
      type: "mouse",
      action: "released",
      ...point,
      button: mouseButtonName(e.button),
      buttons: e.buttons,
      click_count: lastPress.current.count,
      modifiers: modifierBits(e),
    });
  };

  // Moves are coalesced to one per animation frame: a fast mouse fires far
  // more events than the browser on the other end needs to see.
  const onPointerMove = (e: React.PointerEvent<HTMLImageElement>) => {
    const point = pagePoint(e.clientX, e.clientY);
    if (!point) return;
    const queued = pendingMove.current !== null;
    pendingMove.current = { ...point, buttons: e.buttons, modifiers: modifierBits(e) };
    if (queued) return;
    requestAnimationFrame(() => {
      const move = pendingMove.current;
      pendingMove.current = null;
      if (!move) return;
      send({
        type: "mouse",
        action: "moved",
        x: move.x,
        y: move.y,
        buttons: move.buttons,
        button: move.buttons & 1 ? "left" : "none",
        modifiers: move.modifiers,
      });
    });
  };

  // React's wheel handler is passive and can't stop the outer page scrolling,
  // so this one is attached by hand.
  useEffect(() => {
    const img = imgRef.current;
    if (!img) return;
    const onWheel = (e: WheelEvent) => {
      const point = pagePoint(e.clientX, e.clientY);
      if (!point) return;
      e.preventDefault();
      const scale = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? 800 : 1;
      send({
        type: "wheel",
        ...point,
        delta_x: e.deltaX * scale,
        delta_y: e.deltaY * scale,
        modifiers: modifierBits(e),
      });
    };
    img.addEventListener("wheel", onWheel, { passive: false });
    return () => img.removeEventListener("wheel", onWheel);
  }, [pagePoint, send]);

  const onKey = (action: "down" | "up") => (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // IME composition and phone keyboards ("Unidentified") arrive through
    // `input` as finished text instead.
    if (e.nativeEvent.isComposing || e.key === "Unidentified" || e.keyCode === 229) return;
    // Let paste through to the `paste` event, which has the clipboard text.
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v") return;
    e.preventDefault();
    const id = e.code || e.key;
    if (action === "down") keysDown.current.add(id);
    else if (!keysDown.current.delete(id)) return;
    send({
      type: "key",
      action,
      key: e.key,
      code: e.code,
      key_code: e.keyCode,
      modifiers: modifierBits(e),
    });
  };

  const onKeyboardInput = (e: React.FormEvent<HTMLTextAreaElement>) => {
    const target = e.currentTarget;
    if ((e.nativeEvent as InputEvent).isComposing) return;
    if (target.value) send({ type: "text", text: target.value });
    target.value = "";
  };

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    e.preventDefault();
    const text = e.clipboardData.getData("text/plain");
    if (text) send({ type: "text", text });
  };

  const navigate = (e: React.FormEvent) => {
    e.preventDefault();
    const url = address.trim();
    if (url) send({ type: "navigate", url });
    setEditingAddress(false);
    keyboardRef.current?.focus({ preventScroll: true });
  };

  const live = status === "live";

  return (
    <div className="flex h-full flex-col bg-surface">
      <div className="flex flex-wrap items-center gap-1.5 border-b border-border px-3 py-2">
        <ToolbarButton label="Back" disabled={!live} onClick={() => send({ type: "back" })}>
          <ArrowLeftIcon className="h-4 w-4" />
        </ToolbarButton>
        <ToolbarButton label="Forward" disabled={!live} onClick={() => send({ type: "forward" })}>
          <ArrowRightIcon className="h-4 w-4" />
        </ToolbarButton>
        <ToolbarButton label="Reload" disabled={!live} onClick={() => send({ type: "reload" })}>
          <ArrowPathIcon className="h-4 w-4" />
        </ToolbarButton>
        <form onSubmit={navigate} className="min-w-0 flex-1">
          <input
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            onFocus={() => setEditingAddress(true)}
            onBlur={() => setEditingAddress(false)}
            disabled={!live}
            placeholder="Address"
            aria-label="Address"
            spellCheck={false}
            className="w-full rounded-lg border border-border bg-surface-secondary px-2.5 py-1 text-sm text-text-primary outline-none focus:border-accent disabled:opacity-50"
          />
        </form>
        <button
          type="button"
          disabled={!live}
          onClick={() => keyboardRef.current?.focus()}
          className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary transition hover:border-accent hover:text-accent disabled:opacity-50 md:hidden"
        >
          Keyboard
        </button>
        <StatusPill status={status} />
      </div>

      {tabs.length > 1 && (
        <div className="flex gap-1 overflow-x-auto border-b border-border px-3 py-1.5">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              type="button"
              onClick={() => send({ type: "switch_tab", id: tab.id })}
              title={tab.url}
              className={`max-w-[14rem] shrink-0 truncate rounded-md px-2 py-0.5 text-xs transition ${
                tab.id === currentTab
                  ? "bg-surface-tertiary text-text-primary"
                  : "text-text-tertiary hover:text-text-secondary"
              }`}
            >
              {tab.title || tab.url || "New tab"}
            </button>
          ))}
        </div>
      )}

      <div className="relative min-h-0 flex-1 bg-surface-secondary">
        <img
          ref={imgRef}
          alt="Live view of the agent's browser"
          draggable={false}
          onPointerDown={onPointerDown}
          onPointerUp={onPointerUp}
          onPointerMove={onPointerMove}
          onContextMenu={(e) => e.preventDefault()}
          className={`h-full w-full touch-none select-none object-contain ${
            !hasFrame ? "invisible" : live ? "cursor-default" : "pointer-events-none opacity-40"
          }`}
        />
        {/* Keyboard sink: keeps focus so keys, pastes and phone-keyboard text
            can be forwarded, without the page itself ever scrolling or typing. */}
        <textarea
          ref={keyboardRef}
          aria-label="Type into the browser"
          autoCapitalize="off"
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          onKeyDown={onKey("down")}
          onKeyUp={onKey("up")}
          onInput={onKeyboardInput}
          onCompositionEnd={onKeyboardInput}
          onPaste={onPaste}
          className="absolute bottom-0 left-0 h-px w-px opacity-0"
        />

        {(!hasFrame || status !== "live") && (
          <div className="absolute inset-0 flex items-center justify-center p-6">
            <div className="max-w-sm rounded-xl border border-border bg-surface px-5 py-4 text-center shadow-sm">
              {status === "ended" ? (
                <>
                  <p className="text-sm text-text-primary">{endMessage}</p>
                  <button
                    type="button"
                    onClick={() => {
                      setHasFrame(false);
                      setAttempt((n) => n + 1);
                    }}
                    className="mt-3 rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary transition hover:border-accent hover:text-accent"
                  >
                    Reconnect
                  </button>
                </>
              ) : (
                <p className="text-sm text-text-secondary">Connecting to the browser…</p>
              )}
            </div>
          </div>
        )}

        {error && (
          <div className="absolute left-1/2 top-3 -translate-x-1/2 rounded-lg bg-danger-bg px-3 py-1.5 text-xs text-danger-text shadow-sm">
            {error}
          </div>
        )}
      </div>

      <p className="border-t border-border px-3 py-1.5 text-xs text-text-tertiary">
        You&apos;re controlling the agent&apos;s browser. When you&apos;re done, go back to the
        chat and choose <span className="text-text-secondary">Resume Agent</span>.
      </p>
    </div>
  );
}

function ToolbarButton({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className="rounded-lg p-1.5 text-text-secondary transition hover:bg-surface-tertiary hover:text-text-primary disabled:opacity-40"
    >
      {children}
    </button>
  );
}

function StatusPill({ status }: { status: Status }) {
  const styles: Record<Status, string> = {
    connecting: "bg-surface-tertiary text-text-tertiary",
    live: "bg-success-bg text-success-text",
    ended: "bg-surface-tertiary text-text-tertiary",
  };
  const labels: Record<Status, string> = {
    connecting: "Connecting",
    live: "Live",
    ended: "Ended",
  };
  return (
    <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${styles[status]}`}>
      {labels[status]}
    </span>
  );
}
