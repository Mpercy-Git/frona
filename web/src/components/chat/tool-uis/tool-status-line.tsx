"use client";

import { CheckCircleIcon, XCircleIcon, QuestionMarkCircleIcon, KeyIcon, ServerIcon, WrenchScrewdriverIcon } from "@heroicons/react/24/outline";
import { useWizardAnswers } from "@/lib/wizard-answers-context";
import type { ReactNode } from "react";

interface ToolStatusLineProps {
  toolCallId: string;
  /** Icon to show when pending (no answer yet) */
  pendingIcon?: ReactNode;
  /** One-line label text (e.g. question text, "Credential: home assistant") */
  label: string;
  /** Server-confirmed status */
  serverStatus: string;
  /** Server-confirmed result or response */
  serverAnswer?: string | null;
}

export function ToolStatusLine({ toolCallId, pendingIcon, label, serverStatus, serverAnswer }: ToolStatusLineProps) {
  const wizardAnswers = useWizardAnswers();
  const wizardAnswer = wizardAnswers.get(toolCallId);
  const localAnswer = wizardAnswer?.response;

  const denied = serverStatus === "denied" || wizardAnswer?.action === "fail";
  const resolved = !denied && (serverStatus === "resolved" || serverAnswer != null || !!localAnswer);
  const displayAnswer = serverAnswer ?? localAnswer;
  const isLocal = !serverAnswer && !!localAnswer;

  const preview = label.length > 80 ? label.slice(0, 80) + "..." : label;

  if (denied) {
    return (
      <div className="tool-status-item mt-1 flex items-start gap-2 rounded-lg bg-surface-tertiary/50 px-3 py-2 text-sm">
        <XCircleIcon className="mt-0.5 h-4 w-4 shrink-0 text-text-tertiary" />
        <div className="min-w-0 break-all">
          <span className="text-text-tertiary">{preview}</span>
          <span className="ml-1 text-text-tertiary italic">→ {displayAnswer ?? "Denied"}</span>
        </div>
      </div>
    );
  }

  if (resolved) {
    return (
      <div className="tool-status-item mt-1 flex items-start gap-2 rounded-lg bg-surface-tertiary/50 px-3 py-2 text-sm">
        <CheckCircleIcon className="mt-0.5 h-4 w-4 shrink-0 text-accent" />
        <div className="min-w-0 break-all">
          <span className="text-text-tertiary">{preview}</span>
          {displayAnswer && (
            <span className="ml-1 font-medium text-text-primary">→ {displayAnswer}</span>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="tool-status-item mt-1 flex items-start gap-2 rounded-lg bg-surface-tertiary/30 px-3 py-2 text-sm">
      {pendingIcon ?? <QuestionMarkCircleIcon className="mt-0.5 h-4 w-4 shrink-0 text-text-tertiary" />}
      <div className="min-w-0 break-all">
        <span className="text-text-tertiary">{preview}</span>
        <span className="ml-1 text-text-quaternary italic">awaiting answer</span>
      </div>
    </div>
  );
}

/** Helper to pick the right pending icon by tool type */
export function toolPendingIcon(type?: string) {
  switch (type) {
    case "Question":
      return <QuestionMarkCircleIcon className="mt-0.5 h-4 w-4 shrink-0 text-text-tertiary" />;
    case "VaultApproval":
      return <KeyIcon className="mt-0.5 h-4 w-4 shrink-0 text-text-tertiary" />;
    case "ServiceApproval":
      return <ServerIcon className="mt-0.5 h-4 w-4 shrink-0 text-text-tertiary" />;
    case "HumanInTheLoop":
      return <WrenchScrewdriverIcon className="mt-0.5 h-4 w-4 shrink-0 text-text-tertiary" />;
    default:
      return <QuestionMarkCircleIcon className="mt-0.5 h-4 w-4 shrink-0 text-text-tertiary" />;
  }
}
