"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const TakeoverToolUI = makeAssistantToolUI<{ prompt: string; url: string; status: string; reason: string; debugger_url: string }, string>({
  toolName: "Takeover",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("Takeover")}
      label={args.reason}
      serverStatus={args.status}
      serverAnswer={result ?? null}
    />
  ),
});
