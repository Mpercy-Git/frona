"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const CredentialToolUI = makeAssistantToolUI<{ prompt: string; url: string; status: string; query: string; reason: string }, string>({
  toolName: "Credential",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("Credential")}
      label={`Credential: ${args.query}`}
      serverStatus={args.status}
      serverAnswer={args.status === "resolved" ? "Granted" : result ?? null}
    />
  ),
});

/** Batched credential request (app key + user key + …). `items` arrives as a
 * JSON string because assistant-ui args are flattened to primitives. */
export const CredentialsToolUI = makeAssistantToolUI<{ prompt: string; url: string; status: string; items: string; reason: string }, string>({
  toolName: "Credentials",
  render: ({ args, result, toolCallId }) => {
    let count = 0;
    try {
      const parsed = JSON.parse(args.items ?? "[]");
      if (Array.isArray(parsed)) count = parsed.length;
    } catch {
      // leave count at 0
    }
    const label = count > 0 ? `Credentials: ${count} requested` : "Credentials requested";
    return (
      <ToolStatusLine
        toolCallId={toolCallId}
        pendingIcon={toolPendingIcon("Credential")}
        label={label}
        serverStatus={args.status}
        serverAnswer={args.status === "resolved" ? "Granted" : result ?? null}
      />
    );
  },
});
