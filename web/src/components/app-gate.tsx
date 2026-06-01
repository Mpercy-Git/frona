"use client";

import { useEffect, useRef } from "react";
import { useAuth } from "@/lib/auth";
import { useNavigationRaw } from "@/lib/navigation-context";
import { BootSpinner, RequireAuth } from "@/components/require-auth";

/// When this renders children, the user is authed, connection is online,
/// and all nav boot fields (`agents`, `spaces`, `tasks`, `contacts`) are
/// defined.
export function AppGate({ children }: { children: React.ReactNode }) {
  return (
    <RequireAuth>
      <NavigationGate>{children}</NavigationGate>
    </RequireAuth>
  );
}

function NavigationGate({ children }: { children: React.ReactNode }) {
  const { user } = useAuth();
  const { agents, spaces, tasks, contacts, refresh } = useNavigationRaw();

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    if (!user) return;
    refreshRef.current().catch(() => {
      // Network failures flip connectionState to "offline" via auth.ts;
      // RequireAuth handles the retry.
    });
  }, [user]);

  const dataReady =
    agents !== undefined &&
    spaces !== undefined &&
    tasks !== undefined &&
    contacts !== undefined;

  if (dataReady) {
    return <>{children}</>;
  }

  const pending: string[] = [];
  if (agents === undefined) pending.push("agents");
  if (spaces === undefined) pending.push("chats");
  if (tasks === undefined) pending.push("tasks");
  if (contacts === undefined) pending.push("contacts");
  const subtitle =
    pending.length === 0
      ? "Loading your workspace…"
      : `Loading ${pending.join(", ")}…`;

  return <BootSpinner subtitle={subtitle} />;
}
