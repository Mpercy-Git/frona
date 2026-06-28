"use client";

import { useState, useEffect, useCallback } from "react";
import { api } from "./api-client";

type PermissionState = "default" | "granted" | "denied" | "unsupported";

export function usePushNotifications() {
  const [permission, setPermission] = useState<PermissionState>("default");
  const [subscribed, setSubscribed] = useState(false);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (!("Notification" in window) || !("serviceWorker" in navigator)) {
      setPermission("unsupported");
      return;
    }
    setPermission(Notification.permission as PermissionState);

    // Check if already subscribed.
    navigator.serviceWorker.ready
      .then((reg) => reg.pushManager.getSubscription())
      .then((sub) => {
        if (sub) setSubscribed(true);
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

  return { permission, subscribed, loading, enable, disable };
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