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
  const options = {
    body: data.body || "",
    icon: "/icon-192.png",
    badge: "/badge-72.png",
    tag: data.id || undefined,
    data: {
      url: data.url || "/",
      id: data.id,
    },
  };

  event.waitUntil(self.registration.showNotification(title, options));
});

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