"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const ServiceApprovalToolUI = makeAssistantToolUI<{ action: string; manifest: Record<string, unknown>; previous_manifest: Record<string, unknown> | null; status: string; response: string | null }, string>({
  toolName: "ServiceApproval",
  render: ({ args, result, toolCallId }) => {
    const name = String(args.manifest?.name || args.manifest?.id || "service");
    return (
      <ToolStatusLine
        toolCallId={toolCallId}
        pendingIcon={toolPendingIcon("ServiceApproval")}
        label={`${args.action}: ${name}`}
        serverStatus={args.status}
        serverAnswer={args.status === "resolved" ? "Approved" : result ?? args.response}
      />
    );
  },
});
