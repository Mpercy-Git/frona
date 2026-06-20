import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { DeleteTaskView } from "../delete-task";
import { mkProps } from "./helpers";

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy({}, {
    get: () => (props: Record<string, unknown>) => {
      const { children, ...rest } = props as { children?: React.ReactNode } & Record<string, unknown>;
      const filtered: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(rest)) {
        if (k === "initial" || k === "animate" || k === "exit" || k === "transition") continue;
        filtered[k] = v;
      }
      return <div {...filtered}>{children}</div>;
    },
  }),
}));

describe("DeleteTaskView", () => {
  const taskId = "019ec43f-9ac0-786e-a7f2-a7e9b0f631a8";

  it("uses the task title from the result message as subtitle", () => {
    render(
      <DeleteTaskView
        {...mkProps({
          toolName: "delete_task",
          args: { task_id: taskId },
          result: JSON.stringify({
            message: "Task 'Drink water reminder' cancelled.",
          }),
        })}
      />,
    );
    expect(screen.getByText("Delete Task")).toBeInTheDocument();
    expect(screen.getByText("— Drink water reminder")).toBeInTheDocument();
  });

  it("falls back to short task_id when the result hasn't arrived", () => {
    render(
      <DeleteTaskView
        {...mkProps({
          toolName: "delete_task",
          args: { task_id: taskId },
          result: undefined,
        })}
      />,
    );
    expect(screen.getByText("— 019ec43f")).toBeInTheDocument();
  });

  it("renders the confirmation message in the body", () => {
    render(
      <DeleteTaskView
        {...mkProps({
          toolName: "delete_task",
          args: { task_id: taskId },
          result: JSON.stringify({
            message: "Task 'Drink water reminder' cancelled.",
          }),
        })}
      />,
    );
    expect(
      screen.getByText("Task 'Drink water reminder' cancelled."),
    ).toBeInTheDocument();
  });

  it("renders the full task_id in monospace", () => {
    render(
      <DeleteTaskView
        {...mkProps({
          toolName: "delete_task",
          args: { task_id: taskId },
          result: JSON.stringify({
            message: "Task 'X' cancelled.",
          }),
        })}
      />,
    );
    expect(screen.getByText(`ID: ${taskId}`)).toBeInTheDocument();
  });

  it("handles titles that contain single quotes (greedy match)", () => {
    render(
      <DeleteTaskView
        {...mkProps({
          toolName: "delete_task",
          args: { task_id: taskId },
          result: JSON.stringify({
            message: "Task 'Don't forget' cancelled.",
          }),
        })}
      />,
    );
    expect(screen.getByText("— Don't forget")).toBeInTheDocument();
  });

  it("disables expansion when there's no task_id and no message", () => {
    render(
      <DeleteTaskView
        {...mkProps({
          toolName: "delete_task",
          args: {},
          result: undefined,
        })}
      />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });
});
