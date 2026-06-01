"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const HumanInTheLoopToolUI = makeAssistantToolUI<{ reason: string; debugger_url: string; status: string; response: string | null }, string>({
  toolName: "HumanInTheLoop",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("HumanInTheLoop")}
      label={args.reason}
      serverStatus={args.status}
      serverAnswer={result ?? args.response}
    />
  ),
});
