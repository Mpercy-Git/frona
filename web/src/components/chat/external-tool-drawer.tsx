"use client";

import { useState, useCallback, useContext, useEffect, useRef } from "react";
import {
  QuestionMarkCircleIcon,
  KeyIcon,
  ServerIcon,
  WrenchScrewdriverIcon,
  ForwardIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  ChevronUpIcon,
  ChevronDownIcon,
} from "@heroicons/react/24/outline";
import type { ToolCall } from "@/lib/types";
import { usePendingTools } from "@/lib/pending-tools-context";
import { api } from "@/lib/api-client";
import { ChatContext } from "@/lib/chat-context";
import { ToolContentDispatch } from "./tool-uis/tool-content";

function toolIcon(te: ToolCall) {
  switch (te.tool_data?.type) {
    case "Question":
      return <QuestionMarkCircleIcon className="h-5 w-5 text-accent" />;
    case "VaultApproval":
      return <KeyIcon className="h-5 w-5 text-warning" />;
    case "ServiceApproval":
      return <ServerIcon className="h-5 w-5 text-success" />;
    default:
      return <WrenchScrewdriverIcon className="h-5 w-5 text-text-secondary" />;
  }
}

function toolTitle(te: ToolCall): string {
  switch (te.tool_data?.type) {
    case "Question":
      return "Question";
    case "VaultApproval":
      return "Credential Request";
    case "ServiceApproval":
      return "Service Deployment";
    case "HumanInTheLoop":
      return "Manual Action Required";
    default:
      return "Approval Required";
  }
}



export interface WizardAnswer {
  response: string;
  action: "success" | "fail";
  /** Async callback the wizard runs after hiding — for tools that need their own API call (e.g. vault/service). */
  callback?: () => Promise<void>;
}

/** Stores the wizard's local state — answers + current index + submitted flag + collapsed. */
export function useToolWizard() {
  const [answers, setAnswers] = useState<Map<string, WizardAnswer>>(() => new Map());
  const [currentIndex, setCurrentIndex] = useState(0);
  const [submitted, setSubmitted] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  return { answers, setAnswers, currentIndex, setCurrentIndex, submitted, setSubmitted, collapsed, setCollapsed };
}

export type ToolWizardState = ReturnType<typeof useToolWizard>;

export function ExternalToolDrawer({ wizard }: { wizard: ToolWizardState }) {
  const { answers, setAnswers, currentIndex, setCurrentIndex, submitted, setSubmitted, collapsed, setCollapsed } = wizard;
  const pendingTools = usePendingTools();
  const chatCtx = useContext(ChatContext);
  const chatId = chatCtx?.chatId;

  // Reset wizard state only when genuinely NEW pending tools arrive (different IDs)
  const pendingIds = pendingTools.map((t) => t.id).join(",");
  const prevPendingIdsRef = useRef(pendingIds);
  useEffect(() => {
    const prev = prevPendingIdsRef.current;
    prevPendingIdsRef.current = pendingIds;
    // Only reset when new tools appear — not when tools disappear (after submit)
    if (pendingIds && pendingIds !== prev) {
      setSubmitted(false);
      setAnswers(new Map());
      setCurrentIndex(0);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only reset when IDs change to new values
  }, [pendingIds]);

  const total = pendingTools.length;
  const safeIndex = Math.min(currentIndex, Math.max(0, total - 1));
  const currentTool = pendingTools[safeIndex];

  const submitAll = useCallback(async (finalAnswers: Map<string, WizardAnswer>) => {
    if (!chatId || total === 0) return;
    setSubmitted(true);

    // Run tool-specific callbacks (vault/service approvals)
    const callbacks = pendingTools
      .map((te) => finalAnswers.get(te.id)?.callback)
      .filter((cb): cb is () => Promise<void> => !!cb);
    await Promise.all(callbacks.map((cb) => cb()));

    const resolutions = pendingTools
      .filter((te) => !finalAnswers.get(te.id)?.callback)
      .map((te) => {
        const answer = finalAnswers.get(te.id);
        return {
          tool_call_id: te.id,
          response: answer?.response ?? "User declined to answer",
          action: answer?.action ?? "fail",
        };
      });
    if (resolutions.length > 0) {
      api.post(`/api/chats/${chatId}/tool-calls/resolve`, { resolutions });
    }
  }, [chatId, pendingTools, total, setSubmitted]);

  const handleAnswer = useCallback(
    (response: string, action: "success" | "fail" = "success", callback?: () => Promise<void>) => {
      if (!currentTool) return;
      const nextAnswers = new Map(answers);
      nextAnswers.set(currentTool.id, { response, action, callback });
      setAnswers(nextAnswers);

      const allNowAnswered = pendingTools.every((te) => nextAnswers.has(te.id));
      if (allNowAnswered) {
        submitAll(nextAnswers);
      } else if (safeIndex < total - 1) {
        setCurrentIndex(safeIndex + 1);
      }
    },
    [currentTool, answers, setAnswers, pendingTools, safeIndex, total, setCurrentIndex, submitAll],
  );

  const handleSkipAll = useCallback(async () => {
    if (!chatId || total === 0) return;
    setSubmitted(true);
    api.post(`/api/chats/${chatId}/tool-calls/resolve`, {
      resolutions: pendingTools.map((te) => ({
        tool_call_id: te.id,
        response: "User declined to answer",
        action: "fail",
      })),
    });
  }, [chatId, pendingTools, total, setSubmitted]);

  if (!currentTool || submitted) return null;

  const currentAnswer = answers.get(currentTool.id);
  const isLast = safeIndex === total - 1;
  const isFirst = safeIndex === 0;

  if (collapsed) return null;

  return (
    <div className="tool-drawer group/drawer relative px-4 pt-3 pb-2">
      {/* Invisible hover zone extending above so the pill is reachable */}
      <div className="absolute inset-x-0 -top-4 h-5" />
      {/* Collapse pill — centered at the top, visible on hover */}
      <button
        onClick={() => setCollapsed(true)}
        className="absolute left-1/2 -translate-x-1/2 -top-3 z-10 flex items-center justify-center h-6 w-6 rounded-full border border-border bg-surface shadow-sm text-text-tertiary hover:text-text-primary hover:bg-surface-secondary transition opacity-0 group-hover/drawer:opacity-100"
        title="Collapse"
      >
        <ChevronDownIcon className="h-3 w-3" />
      </button>

      <div className="space-y-2">
        {/* Header with navigation */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            {toolIcon(currentTool)}
            <span className="text-xs font-medium text-text-secondary uppercase tracking-wide">
              {toolTitle(currentTool)}
            </span>
            {total > 1 && (
              <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[10px] font-medium text-text-tertiary">
                {safeIndex + 1} of {total}
              </span>
            )}
          </div>
          <div className="flex items-center gap-1">
            {total > 1 && (
              <>
                <button
                  onClick={() => setCurrentIndex((i) => Math.max(0, i - 1))}
                  disabled={isFirst}
                  className="rounded-lg p-1 text-text-tertiary hover:text-text-secondary transition disabled:opacity-30 disabled:cursor-default"
                  title="Back"
                >
                  <ChevronLeftIcon className="h-4 w-4" />
                </button>
                <button
                  onClick={() => setCurrentIndex((i) => Math.min(total - 1, i + 1))}
                  disabled={isLast}
                  className="rounded-lg p-1 text-text-tertiary hover:text-text-secondary transition disabled:opacity-30 disabled:cursor-default"
                  title="Next"
                >
                  <ChevronRightIcon className="h-4 w-4" />
                </button>
              </>
            )}
            <button
              onClick={handleSkipAll}
              className="flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-text-tertiary hover:text-danger hover:bg-danger/10 transition"
            >
              <ForwardIcon className="h-3.5 w-3.5" />
              Skip{total > 1 ? ` all` : ""}
            </button>
          </div>
        </div>

        {/* Tool content */}
        <ToolContentDispatch
          te={currentTool}
          chatId={chatId ?? ""}
          onSuccess={(response, callback) => handleAnswer(response, "success", callback)}
          onFailure={(response, callback) => handleAnswer(response, "fail", callback)}
          selectedAnswer={currentAnswer?.response}
        />

      </div>
    </div>
  );
}

export function CollapsedToolTab({ wizard }: { wizard: ToolWizardState }) {
  const { collapsed, setCollapsed, currentIndex, submitted } = wizard;
  const pendingTools = usePendingTools();

  const total = pendingTools.length;
  const safeIndex = Math.min(currentIndex, Math.max(0, total - 1));
  const currentTool = pendingTools[safeIndex];

  if (!currentTool || submitted || !collapsed) return null;

  return (
    <button
      onClick={() => setCollapsed(false)}
      className="mb-[-1px] flex items-center gap-2 rounded-t-xl border border-b-0 border-border bg-surface-secondary px-4 py-1.5 hover:bg-surface-tertiary transition"
    >
      {toolIcon(currentTool)}
      <span className="text-xs font-medium text-text-secondary uppercase tracking-wide">
        {toolTitle(currentTool)}
      </span>
      {total > 1 && (
        <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[10px] font-medium text-text-tertiary">
          {safeIndex + 1} of {total}
        </span>
      )}
      <ChevronUpIcon className="h-3.5 w-3.5 text-text-tertiary" />
    </button>
  );
}
