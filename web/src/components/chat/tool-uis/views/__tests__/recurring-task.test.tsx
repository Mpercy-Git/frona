import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { humanizeCron, RecurringTaskView } from "../recurring-task";
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

describe("humanizeCron", () => {
  it("delegates to cronstrue for common patterns", () => {
    expect(humanizeCron("*/2 * * * *")).toBe("Every 2 minutes");
    expect(humanizeCron("0 9 * * *")).toBe("At 09:00 AM");
    expect(humanizeCron("0 0 * * 0")).toBe("At 12:00 AM, only on Sunday");
  });

  it("returns null on unparseable input", () => {
    expect(humanizeCron("not a cron expression")).toBeNull();
  });
});

describe("RecurringTaskView", () => {
  const baseArgs = {
    title: "Drink water reminder",
    instruction: "Remind the user to drink water.",
    cron_expression: "*/2 * * * *",
    cron_mode: "singleton",
    cron_concurrency: "replace",
  };
  const baseResult = JSON.stringify({
    task_id: "019ec43f-9ac0-786e-a7f2-a7e9b0f631a8",
    cron_expression: "*/2 * * * *",
    timezone: "America/Los_Angeles",
    next_run_at: "2026-06-14T03:50:00+00:00",
    message: "Cron job 'Drink water reminder' created.",
  });

  it("renders the task title as subtitle and `Schedule Task` as title", () => {
    render(
      <RecurringTaskView
        {...mkProps({ toolName: "create_recurring_task", args: baseArgs, result: baseResult })}
      />,
    );
    expect(screen.getByText("Schedule Task")).toBeInTheDocument();
    expect(screen.getByText("— Drink water reminder")).toBeInTheDocument();
  });

  it("renders the humanized schedule + raw cron expression", () => {
    render(
      <RecurringTaskView
        {...mkProps({ toolName: "create_recurring_task", args: baseArgs, result: baseResult })}
      />,
    );
    expect(screen.getByText("Every 2 minutes")).toBeInTheDocument();
    expect(screen.getByText("(*/2 * * * *)")).toBeInTheDocument();
  });

  it("renders the next run time in the configured timezone", () => {
    render(
      <RecurringTaskView
        {...mkProps({ toolName: "create_recurring_task", args: baseArgs, result: baseResult })}
      />,
    );
    // Intl formatting in jsdom is deterministic; check key fragments rather
    // than the full string so platform differences don't break the test.
    const next = screen.getByText(/Next run:/);
    expect(next.textContent).toMatch(/Jun\b/);
    expect(next.textContent).toMatch(/PDT|PST/);
  });

  it("renders the instruction text", () => {
    render(
      <RecurringTaskView
        {...mkProps({ toolName: "create_recurring_task", args: baseArgs, result: baseResult })}
      />,
    );
    expect(screen.getByText("Instruction")).toBeInTheDocument();
    expect(screen.getByText("Remind the user to drink water.")).toBeInTheDocument();
  });

  it("renders the mode and concurrency settings", () => {
    render(
      <RecurringTaskView
        {...mkProps({ toolName: "create_recurring_task", args: baseArgs, result: baseResult })}
      />,
    );
    expect(screen.getByText("singleton")).toBeInTheDocument();
    expect(screen.getByText("replace")).toBeInTheDocument();
  });

  it("shows the delegated agent when target_agent is set", () => {
    render(
      <RecurringTaskView
        {...mkProps({
          toolName: "create_recurring_task",
          args: { ...baseArgs, target_agent: "Receptionist" },
          result: baseResult,
        })}
      />,
    );
    expect(screen.getByText("Receptionist")).toBeInTheDocument();
  });

  it("falls back to the raw cron expression when humanize fails", () => {
    render(
      <RecurringTaskView
        {...mkProps({
          toolName: "create_recurring_task",
          args: { ...baseArgs, cron_expression: "garbage" },
          result: undefined,
        })}
      />,
    );
    expect(screen.getByText("garbage")).toBeInTheDocument();
  });
});
