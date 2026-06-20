import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { UpdateIdentityView } from "../update-identity";
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

describe("UpdateIdentityView", () => {
  it("renders 'Identity' as title", () => {
    render(
      <UpdateIdentityView
        {...mkProps({
          toolName: "update_identity",
          args: { attributes: { name: "Mina" } },
        })}
      />,
    );
    expect(screen.getByText("Identity")).toBeInTheDocument();
  });

  it("shows attribute keys as comma-separated subtitle", () => {
    render(
      <UpdateIdentityView
        {...mkProps({
          toolName: "update_identity",
          args: { attributes: { name: "Mina", age: "30", city: "PDX" } },
        })}
      />,
    );
    expect(screen.getByText("— name, age, city")).toBeInTheDocument();
  });

  it("renders set attributes as `key = value` in the body", () => {
    render(
      <UpdateIdentityView
        {...mkProps({
          toolName: "update_identity",
          args: { attributes: { name: "Mina", tone: "playful" } },
        })}
      />,
    );
    expect(screen.getByText("name")).toBeInTheDocument();
    expect(screen.getByText("Mina")).toBeInTheDocument();
    expect(screen.getByText("tone")).toBeInTheDocument();
    expect(screen.getByText("playful")).toBeInTheDocument();
  });

  it("renders empty-string values as removed (line-through + 'removed' label)", () => {
    const { container } = render(
      <UpdateIdentityView
        {...mkProps({
          toolName: "update_identity",
          args: { attributes: { old_pref: "" } },
        })}
      />,
    );
    const stricken = container.querySelector(".line-through");
    expect(stricken).not.toBeNull();
    expect(stricken!.textContent).toBe("old_pref");
    expect(screen.getByText("removed")).toBeInTheDocument();
  });

  it("renders both sets and removes in the same call", () => {
    render(
      <UpdateIdentityView
        {...mkProps({
          toolName: "update_identity",
          args: { attributes: { name: "Mina", retired_pref: "" } },
        })}
      />,
    );
    expect(screen.getByText("Mina")).toBeInTheDocument();
    expect(screen.getByText("removed")).toBeInTheDocument();
  });

  it("truncates the subtitle when keys are long", () => {
    const args = {
      attributes: Object.fromEntries(
        Array.from({ length: 20 }, (_, i) => [`attribute_number_${i}`, "x"]),
      ),
    };
    const { container } = render(
      <UpdateIdentityView
        {...mkProps({
          toolName: "update_identity",
          args,
        })}
      />,
    );
    const subtitle = container.querySelector("span.font-normal");
    expect(subtitle).not.toBeNull();
    expect(subtitle!.textContent!.length).toBeLessThanOrEqual(83);
    expect(subtitle!.textContent).toContain("…");
  });

  it("disables expansion when attributes is missing or empty", () => {
    render(
      <UpdateIdentityView
        {...mkProps({
          toolName: "update_identity",
          args: { attributes: {} },
        })}
      />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });
});
