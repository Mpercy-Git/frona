"use client";

import { QuestionToolUI } from "./question-tool-ui";
import { HumanInTheLoopToolUI } from "./human-in-the-loop-tool-ui";
import { VaultApprovalToolUI } from "./vault-approval-tool-ui";
import { ServiceApprovalToolUI } from "./service-approval-tool-ui";
import { TaskCompletionToolUI } from "./task-completion-tool-ui";
import { AttachmentsToolUI } from "./attachments-tool-ui";

export function ToolUIRegistry() {
  return (
    <>
      <QuestionToolUI />
      <HumanInTheLoopToolUI />
      <VaultApprovalToolUI />
      <ServiceApprovalToolUI />
      <TaskCompletionToolUI />
      <AttachmentsToolUI />
    </>
  );
}
