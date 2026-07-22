"use client";

import type { TaskResponse } from "@/lib/types";
import { useNavigation } from "@/lib/navigation-context";
import { TaskItem } from "./task-item";

// Display order: active recurring tasks first (soonest next run first), then
// active one-off tasks, then everything terminal. Without this the list is just
// the API's created_at order, so recurring tasks end up buried among completed
// ones with no sense of when they next run.
function taskRank(task: TaskResponse): number {
  if (task.kind.type === "Cron" && task.status !== "cancelled") return 0; // active recurring
  if (task.status === "pending" || task.status === "inprogress") return 1; // active one-offs
  return 2; // completed / failed / cancelled
}

function nextRunMs(task: TaskResponse): number {
  if (task.kind.type === "Cron" && task.kind.next_run_at) {
    const ms = Date.parse(task.kind.next_run_at);
    if (!Number.isNaN(ms)) return ms;
  }
  return Number.POSITIVE_INFINITY;
}

export function TasksTab() {
  const { tasks } = useNavigation();

  const visible = tasks
    .filter((task) => task.kind.type !== "CronRun")
    .slice()
    .sort((a, b) => {
      const rank = taskRank(a) - taskRank(b);
      if (rank !== 0) return rank;
      // Within the recurring bucket, order by the upcoming run time.
      if (taskRank(a) === 0) return nextRunMs(a) - nextRunMs(b);
      // Other buckets keep the API's created_at order (Array.sort is stable).
      return 0;
    });

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
