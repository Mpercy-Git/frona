"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const AppToolUI = makeAssistantToolUI<{ prompt: string; url: string; status: string; action: string; manifest: string; previous_manifest: string | null }, string>({
  toolName: "App",
  render: ({ args, result, toolCallId }) => {
    let appName = "app";
    try {
      const m = JSON.parse(args.manifest);
      appName = String(m?.name || m?.id || "app");
    } catch {
      // manifest may not be JSON-parseable in tests
    }
    return (
      <ToolStatusLine
        toolCallId={toolCallId}
        pendingIcon={toolPendingIcon("App")}
        label={`${args.action}: ${appName}`}
        serverStatus={args.status}
        serverAnswer={args.status === "resolved" ? "Approved" : result ?? null}
      />
    );
  },
});
