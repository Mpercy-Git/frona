"use client";

import { useState, useEffect, useCallback } from "react";
import { api } from "./api-client";

type PermissionState = "default" | "granted" | "denied" | "unsupported";

/// iOS only delivers Web Push to an installed (home-screen) PWA — in a normal
/// Safari tab `Notification` is undefined, so the hook reports `unsupported`
/// with nothing to act on. This distinguishes "your browser can't" from
/// "install this first", so the UI can say so.
function needsHomeScreenInstall(): boolean {
  if (typeof window === "undefined") return false;
  const ua = window.navigator.userAgent;
  const isIos =
    /iPad|iPhone|iPod/.test(ua) ||
    // iPadOS 13+ reports as a Mac; the touch points give it away.
    (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
  if (!isIos) return false;
  const standalone =
    window.matchMedia("(display-mode: standalone)").matches ||
    // Safari's non-standard flag for home-screen apps.
    (window.navigator as Navigator & { standalone?: boolean }).standalone === true;
  return !standalone;
}

export function usePushNotifications() {
  const [permission, setPermission] = useState<PermissionState>("default");
  const [subscribed, setSubscribed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [installRequired, setInstallRequired] = useState(false);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (!("Notification" in window) || !("serviceWorker" in navigator)) {
      setPermission("unsupported");
      setInstallRequired(needsHomeScreenInstall());
      return;
    }
    setPermission(Notification.permission as PermissionState);

    navigator.serviceWorker.ready
      .then((reg) => reg.pushManager.getSubscription())
      .then(async (sub) => {
        if (!sub) return;
        setSubscribed(true);
        // Re-register the subscription we still hold locally.
        //
        // The browser rotates push subscriptions, and the server prunes an
        // endpoint as soon as a send comes back "gone". The service worker's
        // `pushsubscriptionchange` handler can't re-register on its own — it
        // has no access to the page's in-memory access token — so without this
        // the server can end up with no subscription while the UI still shows
        // notifications as enabled, and nothing ever arrives again.
        //
        // `subscribe` dedupes by endpoint, so re-sending an unchanged
        // subscription is a no-op.
        try {
          await api.post("/api/push/subscribe", sub.toJSON());
        } catch (err) {
          // Not fatal — most likely not signed in yet. The next load retries.
          console.warn("[push] Could not re-sync subscription:", err);
        }
      })
      .catch(() => {});
  }, []);

  const enable = useCallback(async () => {
    if (permission === "unsupported") return;
    setLoading(true);
    try {
      // 1. Request notification permission.
      const result = await Notification.requestPermission();
      setPermission(result as PermissionState);
      if (result !== "granted") return;

      // 2. Fetch VAPID public key from backend.
      const { public_key } = await api.get<{ public_key: string }>(
        "/api/push/vapid-public-key",
      );
      if (!public_key) {
        console.error("[push] VAPID public key not configured on server");
        return;
      }

      // 3. Subscribe via the service worker.
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: urlBase64ToUint8Array(public_key) as BufferSource,
      });

      // 4. POST subscription to backend.
      await api.post("/api/push/subscribe", subscription.toJSON());
      setSubscribed(true);
    } catch (err) {
      console.error("[push] Failed to enable notifications:", err);
    } finally {
      setLoading(false);
    }
  }, [permission]);

  const disable = useCallback(async () => {
    setLoading(true);
    try {
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.getSubscription();
      if (subscription) {
        await api.post("/api/push/unsubscribe", {
          endpoint: subscription.endpoint,
        });
        await subscription.unsubscribe();
      }
      setSubscribed(false);
    } catch (err) {
      console.error("[push] Failed to disable notifications:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  return { permission, subscribed, loading, installRequired, enable, disable };
}

function urlBase64ToUint8Array(base64String: string): Uint8Array {
  const padding = "=".repeat((4 - (base64String.length % 4)) % 4);
  const base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
  const rawData = atob(base64);
  const outputArray = new Uint8Array(rawData.length);
  for (let i = 0; i < rawData.length; ++i) {
    outputArray[i] = rawData.charCodeAt(i);
  }
  return outputArray;
}