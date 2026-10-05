"use client";

import { useEffect, useState } from "react";
import { BellIcon } from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import {
  usePushNotifications,
  type PushTestResult,
} from "@/lib/use-push-notifications";
import { SectionHeader, SectionPanel, Toggle } from "../field";

export function NotificationsSection() {
  const {
    permission,
    subscribed,
    loading,
    installRequired,
    error,
    serverCanSend,
    testing,
    testResult,
    enable,
    disable,
    sendTest,
  } = usePushNotifications();

  // The enable button is driven by whether this device has a *subscription*,
  // not by whether it has permission. Granting permission and then losing the
  // subscription (cleared site data, a rotated endpoint, a failed first
  // register) is common, and gating on permission left that state with no
  // control at all — permanently silent, with nothing to click.
  const canSubscribe =
    !subscribed && permission !== "unsupported" && permission !== "denied";

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

          {permission === "unsupported" && installRequired && (
            <p className="text-sm text-text-secondary">
              To get notifications on iPhone or iPad, add frona to your home
              screen first: tap the Share button, then{" "}
              <span className="font-medium text-text-primary">
                Add to Home Screen
              </span>
              , and open it from there. iOS only delivers notifications to
              installed apps.
            </p>
          )}

          {permission === "unsupported" && !installRequired && (
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

          {serverCanSend === false && permission !== "unsupported" && (
            <p className="text-sm text-error">
              This server has no usable VAPID key pair, so it cannot send push
              notifications to any device. It normally generates one on first
              start and keeps it in{" "}
              <code className="font-mono text-xs">
                {"{data_dir}"}/system/vapid.json
              </code>
              , so check the server log for why that failed — or set{" "}
              <code className="font-mono text-xs">
                FRONA_PUSH_VAPID_PUBLIC_KEY
              </code>{" "}
              and{" "}
              <code className="font-mono text-xs">
                FRONA_PUSH_VAPID_PRIVATE_KEY
              </code>{" "}
              (generate them with{" "}
              <code className="font-mono text-xs">
                npx web-push generate-vapid-keys
              </code>
              ) and restart the server.
            </p>
          )}

          {subscribed && (
            <div className="flex flex-wrap items-center gap-3">
              <span className="text-sm text-success">
                ✓ Notifications enabled on this device
              </span>
              <button
                onClick={sendTest}
                disabled={testing}
                className="text-sm text-accent hover:underline disabled:opacity-50"
              >
                {testing ? "Sending..." : "Send test notification"}
              </button>
              <button
                onClick={disable}
                disabled={loading}
                className="text-sm text-error hover:underline disabled:opacity-50"
              >
                Disable
              </button>
            </div>
          )}

          {canSubscribe && (
            <button
              onClick={enable}
              disabled={loading}
              className="rounded-lg bg-accent px-4 py-2 text-sm text-white transition hover:opacity-90 disabled:opacity-50"
            >
              {loading
                ? "Enabling..."
                : permission === "granted"
                  ? "Re-enable on this device"
                  : "Enable Notifications"}
            </button>
          )}

          {permission === "granted" && canSubscribe && (
            <p className="text-sm text-text-secondary">
              This browser has permission to notify you, but this device has no
              push subscription registered — nothing will arrive until you
              re-enable it.
            </p>
          )}

          {error && <p className="text-sm text-error">{error}</p>}

          {testResult && <TestResult result={testResult} />}
        </div>
      </SectionPanel>

      <PushPreferences />
    </div>
  );
}

interface NotificationPreferences {
  push_approval: boolean;
  push_agent_message: boolean;
  push_chat_reply: boolean;
  push_failure: boolean;
  push_activity: boolean;
}

const PREFERENCE_TOGGLES: {
  key: keyof NotificationPreferences;
  label: string;
  description: string;
}[] = [
  {
    key: "push_approval",
    label: "Approval needed",
    description: "An agent is paused until you authorise something.",
  },
  {
    key: "push_agent_message",
    label: "Agent messages",
    description: "An agent reaches out to you without being asked.",
  },
  {
    key: "push_chat_reply",
    label: "Chat replies",
    description:
      "An agent finishes replying in one of your chats. Not sent while you are looking at that chat.",
  },
  {
    key: "push_failure",
    label: "Failures",
    description: "An app, MCP server or channel fails to start or crashes.",
  },
  {
    key: "push_activity",
    label: "App and report updates",
    description: "Apps deployed or stopped, and cost reports ready.",
  },
];

/// Which kinds of notification are pushed to devices. Everything still shows
/// in the in-app notification list; replies inside delegated sub-tasks and
/// scheduled runs are never notified.
function PushPreferences() {
  const [prefs, setPrefs] = useState<NotificationPreferences | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .get<NotificationPreferences>("/api/notifications/preferences")
      .then(setPrefs)
      .catch(() => setError("Could not load notification preferences."));
  }, []);

  const update = async (key: keyof NotificationPreferences, value: boolean) => {
    if (!prefs) return;
    const previous = prefs;
    const next = { ...prefs, [key]: value };
    setPrefs(next);
    setError(null);
    try {
      setPrefs(
        await api.put<NotificationPreferences>(
          "/api/notifications/preferences",
          next,
        ),
      );
    } catch {
      setPrefs(previous);
      setError("Could not save notification preferences.");
    }
  };

  return (
    <SectionPanel title="What to push">
      <div className="space-y-4">
        <p className="text-sm text-text-secondary">
          Choose what is sent to your devices. Everything still appears in the
          notification list in the app. Replies inside delegated sub-tasks and
          scheduled runs are never notified — you hear about the outcome in
          the chat that started them.
        </p>
        {prefs &&
          PREFERENCE_TOGGLES.map((t) => (
            <Toggle
              key={t.key}
              label={t.label}
              description={t.description}
              value={prefs[t.key]}
              onChange={(v) => update(t.key, v)}
            />
          ))}
        {error && <p className="text-sm text-error">{error}</p>}
      </div>
    </SectionPanel>
  );
}

/// Report what the server did with a test push.
///
/// The point is to split "the push never left the server" from "the push was
/// delivered and your phone stayed quiet" — the second is an OS/browser
/// notification setting, and no amount of retrying in here fixes it.
function TestResult({ result }: { result: PushTestResult }) {
  if (!result.configured) {
    return (
      <p className="text-sm text-error">
        The server has no usable VAPID key pair, so nothing was sent. Check the
        server log — it reports why the key pair could not be generated or
        loaded.
      </p>
    );
  }

  if (result.attempted === 0) {
    return (
      <p className="text-sm text-error">
        The server has no push subscriptions stored for your account. Disable
        and re-enable notifications on this device.
      </p>
    );
  }

  return (
    <div className="space-y-1 text-sm">
      {result.renewed && (
        <p className="text-text-secondary">
          This device&apos;s old push subscription had expired, so it was
          replaced with a new one and the test was sent again:
        </p>
      )}
      <p className={result.delivered > 0 ? "text-success" : "text-error"}>
        Accepted by {result.delivered} of {result.attempted} registered{" "}
        {result.attempted === 1 ? "device" : "devices"}.
      </p>
      {result.delivered > 0 && (
        <p className="text-text-secondary">
          If nothing appeared on this device, the push service took it but the
          system did not show it — check frona&apos;s notification settings in
          Android/browser settings, and that battery optimisation is not
          restricting the browser.
        </p>
      )}
      {result.failures.map((failure, i) => (
        <p key={i} className="text-error">
          {failure.service}: {failure.reason}
        </p>
      ))}
    </div>
  );
}
