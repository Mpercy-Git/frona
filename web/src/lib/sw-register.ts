"use client";

export async function registerServiceWorker(): Promise<ServiceWorkerRegistration | null> {
  if (typeof window === "undefined") return null;
  if (!("serviceWorker" in navigator)) return null;

  try {
    const registration = await navigator.serviceWorker.register("/sw.js", {
      scope: "/",
    });
    console.log("[sw] Service Worker registered:", registration.scope);
    return registration;
  } catch (err) {
    console.error("[sw] Service Worker registration failed:", err);
    return null;
  }
}