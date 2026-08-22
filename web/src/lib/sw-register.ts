"use client";

let reloadingForSw = false;
let foregroundResponderInstalled = false;

/**
 * Answer the service worker's "are you looking at this?" checks.
 *
 * The worker suppresses a push notification only when a page confirms it is
 * foregrounded on the page the push links to. It cannot determine that itself:
 * `WindowClient.visibilityState` stays "visible" on Android when the screen is
 * locked or the app is behind another one, which silently swallowed every push
 * that arrived while the phone was idle. The page can answer accurately, so it
 * does — and if it is backgrounded or frozen this handler simply does not run
 * in time, which the worker reads as "not viewing" and notifies.
 */
function installForegroundResponder() {
  if (foregroundResponderInstalled) return;
  foregroundResponderInstalled = true;

  navigator.serviceWorker.addEventListener("message", (event) => {
    const data = event.data as { type?: string; url?: string } | undefined;
    if (data?.type !== "frona:foreground-check") return;
    const port = event.ports?.[0];
    if (!port) return;
    port.postMessage({ viewing: isViewing(data.url ?? "/") });
  });
}

/** True when this window is focused, visible, and already on `targetUrl`. */
function isViewing(targetUrl: string): boolean {
  try {
    if (document.visibilityState !== "visible") return false;
    if (!document.hasFocus()) return false;

    const target = new URL(targetUrl, window.location.origin);
    if (window.location.pathname !== target.pathname) return false;
    // Chat deep-links identify the conversation by `?id=`; a different chat is
    // a different page as far as notifications are concerned.
    const current = new URLSearchParams(window.location.search);
    return current.get("id") === target.searchParams.get("id");
  } catch {
    return false;
  }
}

export async function registerServiceWorker(): Promise<ServiceWorkerRegistration | null> {
  if (typeof window === "undefined") return null;
  if (!("serviceWorker" in navigator)) return null;

  installForegroundResponder();

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