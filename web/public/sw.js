// Frona Service Worker — Web Push notifications
// Handles push events, notification clicks, and subscription changes.

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
      // Suppress the notification only when the user is already looking at the
      // exact page it links to: a window that is focused/visible AND currently
      // on the same URL (same path + same chat id). In every other case —
      // window unfocused, a different chat open, or a different page — we show
      // it, so an active conversation stays quiet while everything else alerts.
      if (await isViewingTarget(url)) return;
      await self.registration.showNotification(title, options);
    })(),
  );
});

/** True if a focused/visible window is already on `targetUrl`. */
async function isViewingTarget(targetUrl) {
  try {
    const target = new URL(targetUrl, self.location.origin);
    const allClients = await clients.matchAll({
      type: "window",
      includeUncontrolled: true,
    });
    for (const client of allClients) {
      // Only an actively-focused (or at least visible) window counts as
      // "active"; a backgrounded tab should still notify.
      const active =
        client.focused === true || client.visibilityState === "visible";
      if (!active) continue;

      const current = new URL(client.url);
      if (current.origin !== target.origin) continue;
      if (current.pathname !== target.pathname) continue;
      // For chat deep-links, the conversation is identified by `?id=`.
      if (current.searchParams.get("id") !== target.searchParams.get("id")) {
        continue;
      }
      return true;
    }
  } catch {
    // On any parsing/matching error, fall through and show the notification.
  }
  return false;
}

self.addEventListener("notificationclick", (event) => {
  event.notification.close();

  const targetUrl = event.notification.data?.url || "/";

  event.waitUntil(
    (async () => {
      const allClients = await clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      });

      // Focus an existing tab if one is open.
      for (const client of allClients) {
        if (client.url.includes(self.location.origin)) {
          if ("focus" in client) {
            await client.focus();
            if ("navigate" in client) {
              await client.navigate(targetUrl);
            }
          }
          return;
        }
      }

      // No existing tab — open a new one.
      if (clients.openWindow) {
        await clients.openWindow(targetUrl);
      }
    })(),
  );
});

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