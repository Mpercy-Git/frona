// Frona Service Worker — Web Push notifications
// Handles push events, notification clicks, and subscription changes.

// Take over immediately when a new version ships. Without this, an updated
// worker sits in "waiting" until every tab closes, so behaviour changes (like
// the foreground check below) don't apply to existing sessions — the old
// worker keeps running.
self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("push", (event) => {
  let data;
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = { title: "Frona", body: event.data ? event.data.text() : "" };
  }

  const title = data.title || "Frona";
  const url = data.url || "/";
  const options = {
    body: data.body || "",
    icon: "/icon-192.png",
    badge: "/badge-72.png",
    tag: data.id || undefined,
    data: {
      url,
      id: data.id,
    },
  };

  event.waitUntil(
    (async () => {
      // Suppress the notification only when a page positively confirms it is
      // in the foreground AND already showing the exact target of this push.
      // Anything else — no page open, no answer, an answer that says otherwise
      // — shows the notification.
      if (await isViewingTarget(url)) return;
      await self.registration.showNotification(title, options);
    })(),
  );
});

/**
 * True only if an open page *tells us* it is foregrounded on `targetUrl`.
 *
 * The obvious implementation — read `client.focused` / `client.visibilityState`
 * from `clients.matchAll()` — is what this replaced, and it is why Android
 * stopped raising notifications. Chrome on Android leaves a client's
 * `visibilityState` at "visible" in situations where the user plainly cannot
 * see it: the screen is locked, or the PWA/tab is behind another app. Every
 * push that arrived while the phone sat idle on a chat page was therefore
 * treated as "you're already looking at this" and never shown — precisely the
 * moment a push is worth having.
 *
 * So the service worker no longer infers foreground state. It asks, and the
 * page answers with `document.hasFocus()` and its own `location`, which are
 * accurate. A backgrounded or frozen page does not get its handler run in
 * time and simply does not answer, and an unanswered check means "show it".
 * Every failure mode now falls on the side of raising the notification.
 */
async function isViewingTarget(targetUrl) {
  try {
    const allClients = await clients.matchAll({
      type: "window",
      includeUncontrolled: true,
    });
    if (allClients.length === 0) return false;

    const answers = await Promise.all(
      allClients.map((client) => askClient(client, targetUrl)),
    );
    return answers.some((viewing) => viewing === true);
  } catch {
    // On any error, fall through and show the notification.
    return false;
  }
}

/** How long a foregrounded page gets to answer before we just notify. */
const FOREGROUND_CHECK_TIMEOUT_MS = 400;

/** Ask one window whether it is foregrounded on `targetUrl`. */
function askClient(client, targetUrl) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(value);
    };

    // A page that cannot answer within the timeout is not a page the user is
    // looking at (or not one that can tell us), so we notify.
    const timer = setTimeout(() => done(false), FOREGROUND_CHECK_TIMEOUT_MS);

    try {
      const channel = new MessageChannel();
      channel.port1.onmessage = (event) => done(event.data?.viewing === true);
      client.postMessage(
        { type: "frona:foreground-check", url: targetUrl },
        [channel.port2],
      );
    } catch {
      done(false);
    }
  });
}

self.addEventListener("notificationclick", (event) => {
  // Android does not dismiss a notification when it is tapped; only close()
  // does. Do it before anything that can throw.
  event.notification.close();

  const targetUrl = new URL(
    event.notification.data?.url || "/",
    self.location.origin,
  ).href;

  event.waitUntil(
    (async () => {
      await closeNotificationsFor(targetUrl);
      try {
        if (await routeExistingWindow(targetUrl)) return;
      } catch (err) {
        console.warn("[sw] Could not reuse an open window:", err);
      }
      // No usable window — open one. This is also the fallback for every
      // failure above, so a tap always lands somewhere.
      if (clients.openWindow) {
        await clients.openWindow(targetUrl);
      }
    })(),
  );
});

/**
 * Close every notification that points at `targetUrl`. A busy chat can raise
 * several (each push has its own tag), and once the user has opened the chat
 * they are all stale.
 */
async function closeNotificationsFor(targetUrl) {
  try {
    const open = await self.registration.getNotifications();
    for (const n of open) {
      const url = n.data?.url;
      if (url && new URL(url, self.location.origin).href === targetUrl) {
        n.close();
      }
    }
  } catch {
    // Best effort.
  }
}

/**
 * Bring an already-open window to the front and show `targetUrl` in it.
 * Returns false when there is no window to reuse.
 *
 * The previous version focused the first window and then called
 * `client.navigate()`. On Android that window is frequently one this worker
 * does not control (`includeUncontrolled` returns those too, e.g. after a
 * worker update or a Chrome tab beside the installed app), and `navigate()`
 * throws on an uncontrolled client — so the tap did nothing at all. Now the
 * page is asked to route client-side, which works whether or not it is
 * controlled; `navigate()` is only the fallback, and `openWindow()` the
 * fallback after that.
 */
async function routeExistingWindow(targetUrl) {
  const allClients = await clients.matchAll({
    type: "window",
    includeUncontrolled: true,
  });
  const sameOrigin = allClients.filter(
    (c) => new URL(c.url).origin === self.location.origin,
  );
  if (sameOrigin.length === 0) return false;

  // Prefer a window already showing the target, then a focused one, then any.
  const client =
    sameOrigin.find((c) => c.url === targetUrl) ||
    sameOrigin.find((c) => c.focused) ||
    sameOrigin[0];

  let focused = client;
  try {
    focused = (await client.focus()) || client;
  } catch (err) {
    // Chrome refuses focus in some states; opening a window still works.
    console.warn("[sw] focus() failed:", err);
    return false;
  }

  if (focused.url === targetUrl) return true;
  if (await askToNavigate(focused, targetUrl)) return true;

  if ("navigate" in focused) {
    try {
      if (await focused.navigate(targetUrl)) return true;
    } catch {
      // Uncontrolled client — fall through.
    }
  }
  return false;
}

/** How long a focused page gets to acknowledge a navigation request. */
const NAVIGATE_ACK_TIMEOUT_MS = 1000;

/** Ask a page to route itself to `targetUrl`; resolves true once it confirms. */
function askToNavigate(client, targetUrl) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(value);
    };
    const timer = setTimeout(() => done(false), NAVIGATE_ACK_TIMEOUT_MS);
    try {
      const channel = new MessageChannel();
      channel.port1.onmessage = (event) => done(event.data?.ok === true);
      client.postMessage({ type: "frona:navigate", url: targetUrl }, [
        channel.port2,
      ]);
    } catch {
      done(false);
    }
  });
}

// Handle subscription expiration / pushsubscriptionchange.
self.addEventListener("pushsubscriptionchange", (event) => {
  event.waitUntil(
    (async () => {
      const registration = await self.registration;
      const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: await getVapidKey(),
      });
      await sendSubscriptionToServer(subscription);
    })(),
  );
});

async function getVapidKey() {
  const res = await fetch("/api/push/vapid-public-key", {
    credentials: "include",
  });
  const data = await res.json();
  return urlBase64ToUint8Array(data.public_key);
}

async function sendSubscriptionToServer(subscription) {
  await fetch("/api/push/subscribe", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "include",
    body: JSON.stringify(subscription),
  });
}

function urlBase64ToUint8Array(base64String) {
  const padding = "=".repeat((4 - (base64String.length % 4)) % 4);
  const base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
  const rawData = atob(base64);
  const outputArray = new Uint8Array(rawData.length);
  for (let i = 0; i < rawData.length; ++i) {
    outputArray[i] = rawData.charCodeAt(i);
  }
  return outputArray;
}