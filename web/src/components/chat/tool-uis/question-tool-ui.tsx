"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const QuestionToolUI = makeAssistantToolUI<{ prompt: string; url: string; status: string; options: string }, string>({
  toolName: "Question",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("Question")}
      label={args.prompt}
      serverStatus={args.status}
      serverAnswer={result ?? null}
    />
  ),
});
