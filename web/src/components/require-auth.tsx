"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { useAuth } from "@/lib/auth";
import { Logo } from "@/components/logo";

/// Auth/connection gate. Used by AppGate (composes a nav-data wait on top)
/// and standalone by auth-only pages like /setup. Runs an exponential-backoff
/// reconnect loop when offline.
export function RequireAuth({
  children,
  spinnerSubtitle,
}: {
  children: React.ReactNode;
  spinnerSubtitle?: string;
}) {
  const router = useRouter();
  const { connectionState, user, revalidate } = useAuth();
  const [reconnectAttempt, setReconnectAttempt] = useState(0);

  useEffect(() => {
    if (connectionState === "unauthenticated") {
      router.replace("/login");
    }
  }, [connectionState, router]);

  useEffect(() => {
    if (connectionState !== "offline") {
      setReconnectAttempt(0);
      return;
    }
    const delayMs = Math.min(1000 * 2 ** reconnectAttempt, 30_000);
    const timer = setTimeout(async () => {
      try {
        await revalidate();
      } catch {
        // Staying offline schedules another tick.
      }
      setReconnectAttempt((n) => n + 1);
    }, delayMs);
    return () => clearTimeout(timer);
  }, [connectionState, reconnectAttempt, revalidate]);

  if (connectionState === "unauthenticated") {
    return null;
  }

  if (connectionState === "online" && user) {
    return <>{children}</>;
  }

  const subtitle =
    connectionState === "offline"
      ? spinnerSubtitle ??
        (reconnectAttempt > 0
          ? `Reconnecting… (attempt ${reconnectAttempt + 1})`
          : "Reconnecting…")
      : connectionState === "loading"
      ? "Connecting…"
      : undefined;

  return <BootSpinner subtitle={subtitle} />;
}

export function BootSpinner({ subtitle }: { subtitle?: string }) {
  return (
    <div className="flex min-h-screen items-center justify-center">
      <div className="flex flex-col items-center justify-center gap-3">
        <div className="flex items-center justify-center gap-2">
          <Logo size={80} animate />
          <span
            className="text-3xl font-bold text-text-primary tracking-wide"
            style={{ fontFamily: "var(--font-brand)" }}
          >
            FRONA
          </span>
        </div>
        {subtitle && (
          <p className="text-sm text-text-tertiary">{subtitle}</p>
        )}
      </div>
    </div>
  );
}
