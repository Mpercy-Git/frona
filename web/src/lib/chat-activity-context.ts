"use client";

import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  useMemo,
  useRef,
  createElement,
} from "react";
import { getChatActivity } from "./api-client";
import { sseBus, type ChatActivity, type GlobalSSEEvent } from "./sse-event-bus";

export type { ChatActivity };

/// A `working` entry this old with no further events is presumed dead and
/// swept. Nothing else clears it: a server restart, a killed process or a
/// dropped turn never sends the terminal event, and a row that spins forever
/// is worse than one that goes quiet. Generous enough not to fight a long
/// tool call — an agent can legitimately sit inside one for many minutes.
const WORKING_TTL_MS = 15 * 60 * 1000;

const SWEEP_INTERVAL_MS = 60 * 1000;

/// Don't refetch the snapshot more than this often when the tab is flicked
/// back and forth.
const REFETCH_COOLDOWN_MS = 10 * 1000;

/// `idle` is never stored — it is the absence of an entry.
type LiveActivity = Exclude<ChatActivity, "idle">;

interface RemoteEntry {
  activity: LiveActivity;
  /// When this entry was last confirmed, for the staleness sweep.
  at: number;
}

interface ChatActivityContextValue {
  /// What the agent is doing in this chat right now.
  activityOf: (chatId: string | null | undefined) => ChatActivity;
  workingCount: number;
  waitingCount: number;
  /// Publish the state of a *mounted* chat, whose own store knows sooner than
  /// the stream does (the composer marks a turn running the moment you hit
  /// send). While a chat reports here it overrides the stream-derived value.
  setLocalActivity: (chatId: string, activity: ChatActivity) => void;
  /// Stop overriding — the chat unmounted. Deliberately not the same as
  /// reporting `idle`: a chat you navigate away from usually carries on
  /// working, and the stream stays authoritative for it.
  clearLocalActivity: (chatId: string) => void;
}

const ChatActivityContext = createContext<ChatActivityContextValue | null>(null);

export function ChatActivityProvider({ children }: { children: React.ReactNode }) {
  const [remote, setRemote] = useState<Map<string, RemoteEntry>>(() => new Map());
  const [local, setLocal] = useState<Map<string, ChatActivity>>(() => new Map());
  const lastFetchAt = useRef(0);

  const applyEvent = useCallback((chatId: string, activity: ChatActivity) => {
    setRemote((prev) => {
      const existing = prev.get(chatId);
      if (activity === "idle") {
        if (!existing) return prev;
        const next = new Map(prev);
        next.delete(chatId);
        return next;
      }
      // Same state, already fresh enough: skip the re-render. Token events
      // arrive many times a second and would otherwise re-render the whole
      // navigation panel on each one.
      if (existing?.activity === activity && Date.now() - existing.at < SWEEP_INTERVAL_MS) {
        return prev;
      }
      const next = new Map(prev);
      next.set(chatId, { activity, at: Date.now() });
      return next;
    });
  }, []);

  const refresh = useCallback(async () => {
    const startedAt = Date.now();
    lastFetchAt.current = startedAt;
    let snapshot;
    try {
      snapshot = await getChatActivity();
    } catch {
      // Best-effort: keep whatever the stream has told us so far. A failed
      // snapshot degrades the indicators, it never blocks the app.
      return;
    }
    setRemote((prev) => {
      const next = new Map<string, RemoteEntry>();
      for (const chatId of snapshot.working) next.set(chatId, { activity: "working", at: startedAt });
      for (const chatId of snapshot.waiting) {
        if (!next.has(chatId)) next.set(chatId, { activity: "waiting", at: startedAt });
      }
      // Anything the stream reported *while the request was in flight* is
      // newer than the snapshot, so it wins — otherwise a turn that starts
      // during the round-trip is dropped until the next event.
      for (const [chatId, entry] of prev) {
        if (entry.at > startedAt) next.set(chatId, entry);
      }
      return next;
    });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    return sseBus.onGlobal((event: GlobalSSEEvent) => {
      if (event.type === "chat_activity") applyEvent(event.chatId, event.activity);
    });
  }, [applyEvent]);

  // A dropped stream loses every event it didn't deliver, so the snapshot is
  // the only way back to the truth.
  useEffect(() => {
    return sseBus.onReconnect(() => {
      refresh();
    });
  }, [refresh]);

  // Same story for a tab that was backgrounded long enough for the browser to
  // throttle or drop the connection.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState !== "visible") return;
      if (Date.now() - lastFetchAt.current < REFETCH_COOLDOWN_MS) return;
      refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [refresh]);

  // Staleness sweep. `waiting` is deliberately exempt: a chat parked on a
  // human stays parked for as long as it takes them to answer, and it is
  // re-confirmed by every snapshot.
  useEffect(() => {
    const timer = setInterval(() => {
      setRemote((prev) => {
        const cutoff = Date.now() - WORKING_TTL_MS;
        let changed = false;
        const next = new Map(prev);
        for (const [chatId, entry] of prev) {
          if (entry.activity === "working" && entry.at < cutoff) {
            next.delete(chatId);
            changed = true;
          }
        }
        return changed ? next : prev;
      });
    }, SWEEP_INTERVAL_MS);
    return () => clearInterval(timer);
  }, []);

  const setLocalActivity = useCallback((chatId: string, activity: ChatActivity) => {
    setLocal((prev) => {
      if (prev.get(chatId) === activity) return prev;
      const next = new Map(prev);
      next.set(chatId, activity);
      return next;
    });
  }, []);

  const clearLocalActivity = useCallback((chatId: string) => {
    setLocal((prev) => {
      if (!prev.has(chatId)) return prev;
      const next = new Map(prev);
      next.delete(chatId);
      return next;
    });
  }, []);

  const value = useMemo<ChatActivityContextValue>(() => {
    const activityOf = (chatId: string | null | undefined): ChatActivity => {
      if (!chatId) return "idle";
      const mounted = local.get(chatId);
      if (mounted) return mounted;
      return remote.get(chatId)?.activity ?? "idle";
    };

    // Counted over the union of both maps so a chat known only to the open
    // conversation still counts towards the collapsed-rail and mobile dots.
    let workingCount = 0;
    let waitingCount = 0;
    const seen = new Set<string>([...remote.keys(), ...local.keys()]);
    for (const chatId of seen) {
      const activity = activityOf(chatId);
      if (activity === "working") workingCount += 1;
      else if (activity === "waiting") waitingCount += 1;
    }

    return { activityOf, workingCount, waitingCount, setLocalActivity, clearLocalActivity };
  }, [remote, local, setLocalActivity, clearLocalActivity]);

  return createElement(ChatActivityContext.Provider, { value }, children);
}

export function useChatActivity(): ChatActivityContextValue {
  const ctx = useContext(ChatActivityContext);
  if (!ctx) throw new Error("useChatActivity must be used within ChatActivityProvider");
  return ctx;
}

/// Same, for callers that may render outside the provider (the public shared
/// chat page has no navigation to mark up). Returns null instead of throwing.
export function useChatActivityOptional(): ChatActivityContextValue | null {
  return useContext(ChatActivityContext);
}

/// Convenience for a single row: the one chat's state.
export function useActivityOf(chatId: string | null | undefined): ChatActivity {
  return useChatActivity().activityOf(chatId);
}

/// Aggregate for a group of chats — a space row stands in for every chat
/// inside it, since the navigation panel never lists them.
export function useAggregateActivity(chatIds: string[]): ChatActivity {
  const { activityOf } = useChatActivity();
  let waiting = false;
  for (const chatId of chatIds) {
    const activity = activityOf(chatId);
    // Working outranks waiting: it is the state that is changing under you.
    if (activity === "working") return "working";
    if (activity === "waiting") waiting = true;
  }
  return waiting ? "waiting" : "idle";
}
