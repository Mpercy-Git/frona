"use client";

import { useState, useRef, useEffect } from "react";
import { useRouter } from "next/navigation";
import { BellIcon } from "@heroicons/react/24/solid";
import { XMarkIcon } from "@heroicons/react/24/outline";
import { useNotifications } from "@/lib/notification-context";
import { useMobile } from "@/lib/use-mobile";
import { API_URL } from "@/lib/api-client";
import type { Notification, NotificationLevel } from "@/lib/types";
import { formatDistanceToNow } from "date-fns";

function levelColor(level: NotificationLevel): string {
  switch (level) {
    case "success":
      return "text-green-400";
    case "error":
      return "text-red-400";
    case "warning":
      return "text-yellow-400";
    default:
      return "text-blue-400";
  }
}

function levelDot(level: NotificationLevel): string {
  switch (level) {
    case "success":
      return "bg-green-400";
    case "error":
      return "bg-red-400";
    case "warning":
      return "bg-yellow-400";
    default:
      return "bg-blue-400";
  }
}

function timeAgo(dateStr: string): string {
  return formatDistanceToNow(new Date(dateStr), { addSuffix: true });
}

function getNotificationLink(notification: Notification): {
  href: string;
  external: boolean;
} | null {
  if (notification.data.type === "Agent") {
    return { href: `/chat?id=${notification.data.chat_id}`, external: false };
  }
  if (notification.data.type === "App") {
    const { action } = notification.data;
    if (action === "stop" || notification.level === "error") return null;
    return {
      href: `${API_URL}/apps/${notification.data.app_handle}/`,
      external: true,
    };
  }
  return null;
}

function NotificationItem({
  notification,
  onRead,
  onClose,
}: {
  notification: Notification;
  onRead: (id: string) => void;
  onClose: () => void;
}) {
  const router = useRouter();
  const link = getNotificationLink(notification);

  const handleClick = () => {
    if (!notification.read) onRead(notification.id);
    if (link) {
      if (link.external) {
        window.open(link.href, "_blank");
      } else {
        router.push(link.href);
      }
      onClose();
    }
  };

  return (
    <button
      onClick={handleClick}
      className={`w-full text-left px-4 py-3 transition hover:bg-surface-tertiary ${
        notification.read ? "opacity-60" : ""
      } ${link ? "cursor-pointer" : ""}`}
    >
      <div className="flex-1 min-w-0">
        <p
          className={`text-sm font-medium line-clamp-2 ${levelColor(notification.level)}`}
        >
          {notification.title}
        </p>
        {notification.body && (
          <p className="text-xs text-text-secondary line-clamp-2 mt-0.5">
            {notification.body}
          </p>
        )}
        <p className="text-xs text-text-tertiary mt-1 flex items-center gap-1.5">
          <span
            className={`h-1.5 w-1.5 shrink-0 rounded-full ${notification.read ? "bg-text-tertiary" : levelDot(notification.level)}`}
          />
          {timeAgo(notification.created_at)}
        </p>
      </div>
    </button>
  );
}

function NotificationList({ onClose }: { onClose: () => void }) {
  const { notifications, unreadCount, markRead, markAllRead } =
    useNotifications();

  return (
    <>
      <div className="flex items-center justify-between px-4 py-2 border-b border-border shrink-0">
        <span className="text-sm font-medium text-text-secondary">Notifications</span>
        <div className="flex items-center gap-3">
          {unreadCount > 0 && (
            <button
              onClick={() => markAllRead()}
              className="text-xs text-accent hover:text-accent-hover transition cursor-pointer"
            >
              Mark all read
            </button>
          )}
          <button
            onClick={onClose}
            className="flex items-center justify-center h-8 w-8 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition md:hidden"
          >
            <XMarkIcon className="h-5 w-5" />
          </button>
        </div>
      </div>
      <div className="overflow-y-auto flex-1 min-h-0">
        {notifications.length === 0 ? (
          <p className="px-4 py-8 text-sm text-text-tertiary text-center">
            No notifications yet
          </p>
        ) : (
          notifications.map((n) => (
            <NotificationItem
              key={n.id}
              notification={n}
              onRead={markRead}
              onClose={onClose}
            />
          ))
        )}
      </div>
    </>
  );
}

export function NotificationDropdown() {
  const { unreadCount } = useNotifications();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const mobile = useMobile();

  useEffect(() => {
    if (!open || mobile) return;
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open, mobile]);

  if (mobile) {
    return (
      <>
        <button
          onClick={() => setOpen(true)}
          className="relative flex items-center justify-center h-10 w-10 rounded-full bg-surface-tertiary text-text-secondary hover:brightness-125 transition cursor-pointer"
          title="Notifications"
        >
          <BellIcon className="h-5 w-5" />
          {unreadCount > 0 && (
            <span className="absolute -top-2 -right-2 flex items-center justify-center h-5 min-w-5 px-1 rounded-full bg-red-600 text-white text-[11px] font-bold leading-none ring-2 ring-surface-nav">
              {unreadCount > 99 ? "99+" : unreadCount}
            </span>
          )}
        </button>
        {open && (
          <>
            <div className="fixed inset-0 z-[69] bg-black/40" onClick={() => setOpen(false)} />
            <div className="fixed inset-y-0 right-0 z-[70] w-[85vw] max-w-sm bg-surface shadow-xl flex flex-col transition-transform duration-200 ease-out">
              <NotificationList onClose={() => setOpen(false)} />
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
        title="Notifications"
      >
        <BellIcon className="h-5 w-5" />
        {unreadCount > 0 && (
          <span className="absolute -top-2 -right-2 flex items-center justify-center h-5 min-w-5 px-1 rounded-full bg-red-600 text-white text-[11px] font-bold leading-none ring-2 ring-surface-nav">
            {unreadCount > 99 ? "99+" : unreadCount}
          </span>
        )}
      </button>

      {open && (
        <div className="absolute right-0 top-full z-[60] w-80 max-h-[70vh] rounded-xl rounded-tr-none border border-border bg-surface-secondary shadow-lg flex flex-col">
          <div className="absolute -top-px right-0 w-[calc(theme(spacing.10)-3px)] h-[2px] bg-surface-secondary z-[60]" />
          <NotificationList onClose={() => setOpen(false)} />
        </div>
      )}
    </div>
  );
}
