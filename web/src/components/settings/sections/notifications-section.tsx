"use client";

import { BellIcon } from "@heroicons/react/24/outline";
import { usePushNotifications } from "@/lib/use-push-notifications";
import { SectionHeader, SectionPanel } from "../field";

export function NotificationsSection() {
  const { permission, subscribed, loading, enable, disable } = usePushNotifications();

  return (
    <div className="space-y-6">
      <SectionHeader title="Notifications" description="Get native push notifications on this device" icon={BellIcon} />

      <SectionPanel title="Browser Push">
        <div className="space-y-3">
          <p className="text-sm text-text-secondary">
            Native notifications appear on this device even when frona is in the
            background or the tab is closed. Works on desktop Chrome, Firefox,
            Edge, and Android Chrome. iOS requires installing as a PWA.
          </p>

          {permission === "unsupported" && (
            <p className="text-sm text-error">
              Push notifications are not supported in this browser.
            </p>
          )}

          {permission === "denied" && (
            <p className="text-sm text-error">
              Notifications are blocked. Please enable them in your browser
              settings and reload this page.
            </p>
          )}

          {permission === "granted" && subscribed && (
            <div className="flex items-center gap-3">
              <span className="text-sm text-success">
                ✓ Notifications enabled on this device
              </span>
              <button
                onClick={disable}
                disabled={loading}
                className="text-sm text-error hover:underline disabled:opacity-50"
              >
                Disable
              </button>
            </div>
          )}

          {permission !== "granted" &&
            permission !== "unsupported" &&
            permission !== "denied" && (
              <button
                onClick={enable}
                disabled={loading}
                className="rounded-lg bg-accent px-4 py-2 text-sm text-white transition hover:opacity-90 disabled:opacity-50"
              >
                {loading ? "Enabling..." : "Enable Notifications"}
              </button>
            )}
        </div>
      </SectionPanel>
    </div>
  );
}