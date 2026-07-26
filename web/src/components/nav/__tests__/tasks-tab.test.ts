import { describe, it, expect } from "vitest";
import type { TaskResponse } from "@/lib/types";
import { sortTasksForDisplay } from "../tasks-tab";

function mkTask(over: Partial<TaskResponse> & { id: string }): TaskResponse {
  return {
    agent_id: "a-1",
    space_id: null,
    chat_id: null,
    title: over.id,
    description: "",
    status: "pending",
    kind: { type: "Direct", source_chat_id: null },
    run_at: null,
    result_summary: null,
    error_message: null,
    created_at: "2026-07-01T00:00:00Z",
    updated_at: "2026-07-01T00:00:00Z",
    ...over,
  } as TaskResponse;
}

const ids = (tasks: TaskResponse[]) => tasks.map((t) => t.id);

describe("sortTasksForDisplay", () => {
  it("orders one-off tasks by status, not by created_at", () => {
    const sorted = sortTasksForDisplay([
      mkTask({ id: "cancelled", status: "cancelled" }),
      mkTask({ id: "completed", status: "completed" }),
      mkTask({ id: "failed", status: "failed" }),
      mkTask({ id: "pending", status: "pending" }),
      mkTask({ id: "inprogress", status: "inprogress" }),
    ]);

    expect(ids(sorted)).toEqual([
      "inprogress",
      "pending",
      "failed",
      "completed",
      "cancelled",
    ]);
  });

  it("keeps the API's created_at order within one status", () => {
    const sorted = sortTasksForDisplay([
      mkTask({ id: "newest", status: "pending", created_at: "2026-07-03T00:00:00Z" }),
      mkTask({ id: "middle", status: "pending", created_at: "2026-07-02T00:00:00Z" }),
      mkTask({ id: "oldest", status: "pending", created_at: "2026-07-01T00:00:00Z" }),
    ]);

    expect(ids(sorted)).toEqual(["newest", "middle", "oldest"]);
  });

  it("puts live recurring tasks above every one-off, soonest run first", () => {
    const sorted = sortTasksForDisplay([
      mkTask({ id: "running", status: "inprogress" }),
      mkTask({
        id: "cron-later",
        status: "completed",
        kind: { type: "Cron", cron_expression: "0 9 * * *", next_run_at: "2026-07-27T09:00:00Z" },
      }),
      mkTask({
        id: "cron-sooner",
        status: "completed",
        kind: { type: "Cron", cron_expression: "0 8 * * *", next_run_at: "2026-07-26T08:00:00Z" },
      }),
    ] as TaskResponse[]);

    expect(ids(sorted)).toEqual(["cron-sooner", "cron-later", "running"]);
  });

  it("sorts a recurring task with no scheduled run after those that have one", () => {
    const sorted = sortTasksForDisplay([
      mkTask({
        id: "cron-unscheduled",
        status: "pending",
        kind: { type: "Cron", cron_expression: "0 8 * * *", next_run_at: null },
      }),
      mkTask({
        id: "cron-scheduled",
        status: "pending",
        kind: { type: "Cron", cron_expression: "0 9 * * *", next_run_at: "2026-07-27T09:00:00Z" },
      }),
    ] as TaskResponse[]);

    expect(ids(sorted)).toEqual(["cron-scheduled", "cron-unscheduled"]);
  });

  it("demotes a cancelled cron out of the recurring bucket", () => {
    const sorted = sortTasksForDisplay([
      mkTask({
        id: "cron-cancelled",
        status: "cancelled",
        kind: { type: "Cron", cron_expression: "0 9 * * *", next_run_at: "2026-07-27T09:00:00Z" },
      }),
      mkTask({ id: "done", status: "completed" }),
    ] as TaskResponse[]);

    expect(ids(sorted)).toEqual(["done", "cron-cancelled"]);
  });

  it("hides individual cron runs", () => {
    const sorted = sortTasksForDisplay([
      mkTask({
        id: "cron-run",
        status: "completed",
        kind: {
          type: "CronRun",
          source_cron_id: "c-1",
          fire_at: "2026-07-26T08:00:00Z",
          sequence_num: 3,
        },
      }),
      mkTask({ id: "direct", status: "completed" }),
    ] as TaskResponse[]);

    expect(ids(sorted)).toEqual(["direct"]);
  });

  it("sorts an unrecognised status last rather than mid-list", () => {
    const sorted = sortTasksForDisplay([
      mkTask({ id: "mystery", status: "hibernating" }),
      mkTask({ id: "cancelled", status: "cancelled" }),
      mkTask({ id: "running", status: "inprogress" }),
    ]);

    expect(ids(sorted)).toEqual(["running", "cancelled", "mystery"]);
  });

  it("does not mutate the input array", () => {
    const input = [
      mkTask({ id: "completed", status: "completed" }),
      mkTask({ id: "inprogress", status: "inprogress" }),
    ];

    sortTasksForDisplay(input);

    expect(ids(input)).toEqual(["completed", "inprogress"]);
  });
});
