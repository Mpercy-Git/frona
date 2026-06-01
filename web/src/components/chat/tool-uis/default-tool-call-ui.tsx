"use client";

import { memo, useState } from "react";
import { ChevronDownIcon } from "@heroicons/react/24/outline";
import { PuffLoader } from "react-spinners";
import {
  type ToolCallMessagePartStatus,
  type ToolCallMessagePartComponent,
} from "@assistant-ui/react";
import { AnimatePresence, motion } from "motion/react";
import ReactMarkdown from "react-markdown";
import { cn } from "@/lib/utils";
import { useToolTimeline } from "./tool-timeline-context";
import { InlineCode } from "./inline-code";

const ANIMATION_DURATION = 200;

const TOOL_DISPLAY_NAMES: Record<string, string> = {
  web_fetch: "Web Fetch",
  web_search: "Web Search",
  cli: "Terminal",
  python: "Python",
  browser_navigate: "Browser",
  manage_service: "Service",
  request_credentials: "Request Credentials",
  produce_file: "Produce File",
  store_agent_memory: "Remember",
  store_user_memory: "Remember",
  create_task: "Create Task",
  list_tasks: "List Tasks",
  delete_task: "Delete Task",
  task_control: "Task Control",
  complete_task: "Complete Task",
  defer_task: "Defer Task",
  fail_task: "Fail Task",
  update_identity: "Update Identity",
  update_entity: "Update Entity",
  set_heartbeat: "Set Heartbeat",
  notify_human: "Notify",
  request_user_takeover: "Request Takeover",
  ask_user_question: "Ask Question",
  make_voice_call: "Voice Call",
  send_dtmf: "Send DTMF",
  hangup_call: "Hang Up",
};

function displayToolName(name: string): string {
  if (TOOL_DISPLAY_NAMES[name]) return TOOL_DISPLAY_NAMES[name];
  const bare = name.includes("__") ? name.split("__").pop()! : name;
  return bare.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

type ToolStatus = ToolCallMessagePartStatus["type"];


function TimelineDot({
  status,
  isCancelled,
  index,
}: {
  status: ToolStatus;
  isCancelled: boolean;
  index: number;
}) {
  const isRunning = status === "running";
  const isComplete = status === "complete";
  const isFailed = (status === "incomplete" && !isCancelled) || status === "requires-action";

  return (
    <div className="absolute left-0 top-[-3px] z-10 h-6 w-6 flex items-center justify-center">
      <AnimatePresence mode="wait" initial={false}>
        {isRunning ? (
          <motion.div
            key="loader"
            initial={{ opacity: 0, scale: 0.5 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.5 }}
            transition={{ duration: 0.2 }}
          >
            <PuffLoader
              color="var(--text-tertiary)"
              size={24}
              speedMultiplier={0.8}
            />
          </motion.div>
        ) : (
          <motion.div
            key="number"
            initial={{ opacity: 0, scale: 0.5 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.5 }}
            transition={{ duration: 0.2 }}
            className={cn(
              "flex h-6 w-6 items-center justify-center rounded-full text-[10px] font-semibold leading-none",
              isComplete && "bg-success/20 text-success",
              isFailed && "bg-danger/20 text-danger",
              isCancelled && "bg-surface-tertiary text-text-tertiary",
            )}
          >
            {index}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

const ToolFallbackImpl: ToolCallMessagePartComponent = ({
  toolName,
  toolCallId,
  args,
  argsText,
  result,
  status,
}) => {
  const timeline = useToolTimeline();
  const [isOpen, setIsOpen] = useState(false);

  // If timeline context exists, check visibility
  if (timeline && !timeline.isVisible(toolCallId)) return null;

  const isCancelled =
    status?.type === "incomplete" && status.reason === "cancelled";
  const rawDescription =
    typeof args?.description === "string" ? args.description : null;
  const description =
    rawDescription && rawDescription !== toolName ? rawDescription : null;
  const turnText =
    typeof args?.turnText === "string" ? args.turnText : null;
  const isToolError = args?.isError === true;
  const statusType = isToolError ? "incomplete" : (status?.type ?? "complete");
  const isLast = timeline ? timeline.isLastVisible(toolCallId) : false;
  const isFirst = timeline ? timeline.isFirstVisible(toolCallId) : false;
  const toolIndex = timeline ? timeline.getToolIndex(toolCallId) : 0;
  const hiddenCount = timeline?.hiddenCount ?? 0;

  const errorText =
    status?.type === "incomplete"
      ? (() => {
          const error = (status as { error?: unknown }).error;
          if (!error) return null;
          return typeof error === "string" ? error : JSON.stringify(error);
        })()
      : null;

  return (
    <>
      {isFirst && hiddenCount > 0 && (
        <div className="relative pl-8 pb-3 mt-3 flex items-center min-h-6">
          <div className="absolute left-[11px] top-[21px] bottom-0 w-px bg-border" />
          <div className="absolute left-0 z-10 flex h-6 w-6 items-center justify-center rounded-full bg-surface-tertiary text-[10px] font-semibold text-text-tertiary">
            +{hiddenCount}
          </div>
          <span className="text-sm text-text-tertiary leading-none">
            tools used
          </span>
        </div>
      )}
      {turnText && (
        <div className={cn("relative pb-2 flex items-start", isFirst && hiddenCount === 0 && "mt-3")}>
          <div className="absolute left-[11px] top-0 bottom-0 w-px bg-border" />
          <div className="inline-block rounded-r-full bg-surface-tertiary pl-4 pr-3 py-1.5 text-xs text-text-secondary leading-none [&_p]:m-0" style={{ marginLeft: "11px" }}>
            <ReactMarkdown components={{
              pre: ({ children }) => <>{children}</>,
              code: ({ className, children }) => {
                const lang = className?.replace("language-", "");
                const code = String(children).replace(/\n$/, "");
                if (!className) return <code className="text-xs">{children}</code>;
                return <InlineCode code={code} language={lang} />;
              },
            }}>{turnText}</ReactMarkdown>
          </div>
        </div>
      )}
      <motion.div
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: "auto" }}
      exit={{ opacity: 0, height: 0 }}
      transition={{ duration: 0.25, ease: "easeInOut" }}
      className={cn(
        "relative pl-8 pb-2",
        isFirst && hiddenCount === 0 && !turnText && "mt-3",
        isLast && "pb-0",
        isCancelled && "opacity-60",
      )}
    >
      {!isLast && (
        <div className="absolute left-[11px] top-[21px] bottom-0 w-px bg-border" />
      )}

      {/* Status dot with number or spinner */}
      <TimelineDot
        status={statusType}
        isCancelled={!!isCancelled}
        index={toolIndex}
      />

      {/* Trigger */}
      <button
        onClick={() => setIsOpen((v) => !v)}
        className="flex w-full items-center gap-1.5 text-sm transition-colors"
      >
        <span
          className={cn(
            "grow text-left leading-snug",
            isCancelled
              ? "text-text-tertiary line-through"
              : "text-text-secondary",
          )}
        >
          <b>{displayToolName(toolName)}</b>
          {description && (
            <span className="font-normal text-text-tertiary">
              {" "}
              — {description}
            </span>
          )}
        </span>
        <motion.span
          animate={{ rotate: isOpen ? 0 : -90 }}
          transition={{ duration: ANIMATION_DURATION / 1000, ease: "easeOut" }}
          className="shrink-0 text-text-tertiary"
        >
          <ChevronDownIcon className="h-3.5 w-3.5" />
        </motion.span>
      </button>

      {/* Expandable content */}
      <AnimatePresence initial={false}>
        {isOpen && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: ANIMATION_DURATION / 1000, ease: "easeOut" }}
            className="overflow-hidden"
          >
            <div className="mt-2 flex flex-col gap-2 rounded-md border border-border bg-surface-secondary p-3 text-sm">
              {errorText && (
                <div className="text-xs">
                  <p className="font-semibold text-text-tertiary">
                    {isCancelled ? "Cancelled reason:" : "Error:"}
                  </p>
                  <p className="text-text-tertiary">{errorText}</p>
                </div>
              )}
              {argsText && (
                <div className={cn(isCancelled && "opacity-60")}>
                  <pre className="whitespace-pre-wrap text-text-secondary text-xs overflow-x-auto">
                    {argsText}
                  </pre>
                </div>
              )}
              {!isCancelled && result !== undefined && (
                <div className="border-t border-dashed border-border pt-2">
                  <p className="font-semibold text-text-secondary text-xs">Result:</p>
                  <pre className="whitespace-pre-wrap text-text-secondary text-xs overflow-x-auto">
                    {typeof result === "string" ? result : JSON.stringify(result, null, 2)}
                  </pre>
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
    </>
  );
};

export const DefaultToolCallUI = memo(
  ToolFallbackImpl,
) as unknown as ToolCallMessagePartComponent;
DefaultToolCallUI.displayName = "ToolFallback";
