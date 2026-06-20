import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { HeartbeatView, humanizeInterval } from "../heartbeat";
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

describe("humanizeInterval", () => {
  it.each<[number, string]>([
    [0, "Disabled"],
    [1, "Every minute"],
    [5, "Every 5 minutes"],
    [59, "Every 59 minutes"],
    [60, "Every hour"],
    [120, "Every 2 hours"],
    [180, "Every 3 hours"],
    [90, "Every 1h 30m"],
    [125, "Every 2h 5m"],
  ])("humanizes %i minutes as %s", (input, expected) => {
    expect(humanizeInterval(input)).toBe(expected);
  });
});

describe("HeartbeatView", () => {
  it("renders 'Heartbeat' as title and the humanized interval as subtitle", () => {
    render(
      <HeartbeatView
        {...mkProps({
          toolName: "set_heartbeat",
          args: { interval_minutes: 5 },
          result: JSON.stringify({
            message: "x",
            heartbeat_interval: 5,
            next_heartbeat_at: "2026-06-14T03:50:00+00:00",
          }),
        })}
      />,
    );
    expect(screen.getByText("Heartbeat")).toBeInTheDocument();
    expect(screen.getByText("— Every 5 minutes")).toBeInTheDocument();
  });

  it("renders the next heartbeat time when enabled", () => {
    render(
      <HeartbeatView
        {...mkProps({
          toolName: "set_heartbeat",
          args: { interval_minutes: 30 },
          result: JSON.stringify({
            message: "x",
            heartbeat_interval: 30,
            next_heartbeat_at: "2026-06-14T03:50:00+00:00",
          }),
        })}
      />,
    );
    expect(screen.getByText(/Next heartbeat:/)).toBeInTheDocument();
  });

  it("shows 'Disabled' subtitle when interval is 0", () => {
    render(
      <HeartbeatView
        {...mkProps({
          toolName: "set_heartbeat",
          args: { interval_minutes: 0 },
          result: JSON.stringify({
            message: "Heartbeat disabled.",
            heartbeat_interval: null,
            next_heartbeat_at: null,
          }),
        })}
      />,
    );
    expect(screen.getByText("— Disabled")).toBeInTheDocument();
    expect(screen.queryByText(/Next heartbeat:/)).not.toBeInTheDocument();
  });

  it("falls back to args.interval_minutes when result hasn't arrived", () => {
    render(
      <HeartbeatView
        {...mkProps({
          toolName: "set_heartbeat",
          args: { interval_minutes: 15 },
          result: undefined,
        })}
      />,
    );
    expect(screen.getByText("— Every 15 minutes")).toBeInTheDocument();
  });

  it("disables expansion when no interval can be determined", () => {
    render(
      <HeartbeatView
        {...mkProps({
          toolName: "set_heartbeat",
          args: {},
          result: undefined,
        })}
      />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });
});
