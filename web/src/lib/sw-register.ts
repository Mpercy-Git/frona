"use client";

let reloadingForSw = false;

export async function registerServiceWorker(): Promise<ServiceWorkerRegistration | null> {
  if (typeof window === "undefined") return null;
  if (!("serviceWorker" in navigator)) return null;

  // When an UPDATED worker takes control (via skipWaiting + clients.claim),
  // reload once so the page is driven by the up-to-date worker. Skip the
  // first-ever install (no prior controller) to avoid a needless reload, and
  // guard against loops.
  const hadController = !!navigator.serviceWorker.controller;
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (reloadingForSw || !hadController) return;
    reloadingForSw = true;
    window.location.reload();
  });

  try {
    const registration = await navigator.serviceWorker.register("/sw.js", {
      scope: "/",
    });
    console.log("[sw] Service Worker registered:", registration.scope);

    // Proactively check for an updated worker on load, rather than waiting for
    // the browser's periodic (~24h) check — otherwise SW behaviour changes can
    // linger for a day.
    registration.update().catch(() => {});

    return registration;
  } catch (err) {
    console.error("[sw] Service Worker registration failed:", err);
    return null;
  }
}