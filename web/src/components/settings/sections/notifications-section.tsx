"use client";

import { BellIcon } from "@heroicons/react/24/outline";
import {
  usePushNotifications,
  type PushTestResult,
} from "@/lib/use-push-notifications";
import { SectionHeader, SectionPanel } from "../field";

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
    </div>
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
