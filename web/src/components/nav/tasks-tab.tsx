"use client";

import type { TaskResponse } from "@/lib/types";
import { useNavigation } from "@/lib/navigation-context";
import { TaskItem } from "./task-item";

// Display order: active recurring tasks first (soonest next run first), then
// one-off tasks by status — running, then queued, then failures that still want
// attention, then everything finished. Without this the list is just the API's
// created_at order, so recurring tasks end up buried among completed ones with
// no sense of when they next run, and a task that finished last week outranks
// one that is running right now.
const STATUS_RANK: Record<string, number> = {
  inprogress: 1,
  pending: 2,
  failed: 3,
  completed: 4,
  cancelled: 5,
};

// A status the frontend hasn't been taught about sorts after every known one
// instead of silently landing mid-list.
const UNKNOWN_STATUS_RANK = 6;

const RECURRING_RANK = 0;

function taskRank(task: TaskResponse): number {
  // Crons never reach a terminal status on their own, and their own `status`
  // reflects the last run rather than the schedule — while one is live, when it
  // next fires matters more than what it did last.
  if (task.kind.type === "Cron" && task.status !== "cancelled") return RECURRING_RANK;
  return STATUS_RANK[task.status] ?? UNKNOWN_STATUS_RANK;
}

function nextRunMs(task: TaskResponse): number {
  if (task.kind.type === "Cron" && task.kind.next_run_at) {
    const ms = Date.parse(task.kind.next_run_at);
    if (!Number.isNaN(ms)) return ms;
  }
  return Number.POSITIVE_INFINITY;
}

/// Exported for testing; `TasksTab` is the only production caller.
export function sortTasksForDisplay(tasks: TaskResponse[]): TaskResponse[] {
  return tasks
    .filter((task) => task.kind.type !== "CronRun")
    .slice()
    .sort((a, b) => {
      const rank = taskRank(a) - taskRank(b);
      if (rank !== 0) return rank;
      // Within the recurring bucket, order by the upcoming run time.
      if (taskRank(a) === RECURRING_RANK) return nextRunMs(a) - nextRunMs(b);
      // Same status keeps the API's created_at order (Array.sort is stable).
      return 0;
    });
}

export function TasksTab() {
  const { tasks } = useNavigation();

  const visible = sortTasksForDisplay(tasks);

  return (
    <div className="space-y-1 p-2">
      {visible.map((task) => (
        <TaskItem key={task.id} task={task} />
      ))}
      {visible.length === 0 && (
        <p className="px-2 py-4 text-center text-xs text-text-tertiary">
          No active tasks
        </p>
      )}
    </div>
  );
}
