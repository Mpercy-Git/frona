"use client";

import { useRouter } from "next/navigation";
import { makeAssistantToolUI } from "@assistant-ui/react";
import { useSession } from "@/lib/session-context";
import { ArrowTopRightOnSquareIcon, CheckCircleIcon, XCircleIcon } from "@heroicons/react/24/outline";

interface TaskCompletionArgs {
  task_id: string;
  chat_id: string | null;
  status: string;
}

function TaskCompletionContent({ args }: { args: TaskCompletionArgs }) {
  const { activeTaskId } = useSession();
  const router = useRouter();
  const isTaskView = activeTaskId === args.task_id;
  const isError = args.status === "failed";

  if (!isTaskView && !isError) {
    return (
      <button
        onClick={() => router.push(`/chat?task=${args.task_id}`)}
        className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-secondary px-4 py-2 text-sm text-text-secondary cursor-pointer hover:bg-surface-tertiary transition-colors"
      >
        Ask Follow-up Questions
        <ArrowTopRightOnSquareIcon className="h-3.5 w-3.5" />
      </button>
    );
  }

  return (
    <div className="flex items-center gap-1.5 mt-2 text-sm text-text-tertiary">
      {isError ? (
        <XCircleIcon className="h-3.5 w-3.5 text-danger" />
      ) : (
        <CheckCircleIcon className="h-3.5 w-3.5 text-success" />
      )}
      <span>Marked the task as {isError ? "failed" : "completed"}.</span>
    </div>
  );
}

export const TaskCompletionToolUI = makeAssistantToolUI<TaskCompletionArgs, string>({
  // Matches TaskEvent.Completion (from inference::tool_call::TaskEvent).
  // task_event is emitted by complete_task / fail_task tools.
  toolName: "Completion",
  render: ({ args }) => <TaskCompletionContent args={args} />,
});
