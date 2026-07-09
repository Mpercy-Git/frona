"use client";

import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  createElement,
  useRef,
} from "react";
import { api } from "./api-client";
import type { Notification } from "./types";

interface NotificationContextValue {
  notifications: Notification[];
  unreadCount: number;
  markRead: (id: string) => Promise<void>;
  markReadByChat: (chatId: string) => void;
  markAllRead: () => Promise<void>;
  addNotification: (notification: Notification) => void;
}

const NotificationContext = createContext<NotificationContextValue | null>(null);

export function NotificationProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const [unreadCount, setUnreadCount] = useState(0);
  const loadedRef = useRef(false);

  useEffect(() => {
    if (loadedRef.current) return;
    loadedRef.current = true;

    api
      .get<{ notifications: Notification[]; unread_count: number }>(
        "/api/notifications",
      )
      .then((data) => {
        setNotifications(data.notifications);
        setUnreadCount(data.unread_count);
      })
      .catch(() => {});
  }, []);

  const addNotification = useCallback((notification: Notification) => {
    setNotifications((prev) => [notification, ...prev]);
    if (!notification.read) {
      setUnreadCount((prev) => prev + 1);
    }
  }, []);

  const markRead = useCallback(async (id: string) => {
    await api.post("/api/notifications/" + id + "/read", {});
    setNotifications((prev) =>
      prev.map((n) => (n.id === id ? { ...n, read: true } : n)),
    );
    setUnreadCount((prev) => Math.max(0, prev - 1));
  }, []);

  const markReadByChat = useCallback((chatId: string) => {
    setNotifications((prev) => {
      let cleared = 0;
      const next = prev.map((n) => {
        if (!n.read && n.data.type === "Agent" && n.data.chat_id === chatId) {
          cleared++;
          api.post("/api/notifications/" + n.id + "/read", {}).catch(() => {});
          return { ...n, read: true };
        }
        return n;
      });
      if (cleared > 0) setUnreadCount((c) => Math.max(0, c - cleared));
      return cleared > 0 ? next : prev;
    });
  }, []);

  const markAllRead = useCallback(async () => {
    await api.post("/api/notifications/read-all", {});
    setNotifications((prev) => prev.map((n) => ({ ...n, read: true })));
    setUnreadCount(0);
  }, []);

  return createElement(
    NotificationContext.Provider,
    {
      value: {
        notifications,
        unreadCount,
        markRead,
        markReadByChat,
        markAllRead,
        addNotification,
      },
    },
    children,
  );
}

export function useNotifications(): NotificationContextValue {
  const ctx = useContext(NotificationContext);
  if (!ctx)
    throw new Error(
      "useNotifications must be used within NotificationProvider",
    );
  return ctx;
}
