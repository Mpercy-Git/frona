"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const QuestionToolUI = makeAssistantToolUI<{ question: string; options: string[]; status: string; response: string | null }, string>({
  toolName: "Question",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("Question")}
      label={args.question}
      serverStatus={args.status}
      serverAnswer={result ?? args.response}
    />
  ),
});
