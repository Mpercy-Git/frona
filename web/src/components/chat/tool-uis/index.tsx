"use client";

import { QuestionToolUI } from "./question-tool-ui";
import { TakeoverToolUI } from "./human-in-the-loop-tool-ui";
import { CredentialToolUI } from "./credential-tool-ui";
import { AppToolUI } from "./app-tool-ui";
import { TaskCompletionToolUI } from "./task-completion-tool-ui";
import { AttachmentsToolUI } from "./attachments-tool-ui";

export function ToolUIRegistry() {
  return (
    <>
      <QuestionToolUI />
      <TakeoverToolUI />
      <CredentialToolUI />
      <AppToolUI />
      <TaskCompletionToolUI />
      <AttachmentsToolUI />
    </>
  );
}
