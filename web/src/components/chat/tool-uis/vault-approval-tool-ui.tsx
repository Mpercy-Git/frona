"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const VaultApprovalToolUI = makeAssistantToolUI<{ query: string; reason: string; env_var_prefix: string | null; status: string; response: string | null }, string>({
  toolName: "VaultApproval",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("VaultApproval")}
      label={`Credential: ${args.query}`}
      serverStatus={args.status}
      serverAnswer={args.status === "resolved" ? "Approved" : result ?? args.response}
    />
  ),
});
