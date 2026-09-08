import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const config = vi.hoisted(() => ({ getConfig: vi.fn() }));

vi.mock("@/lib/config-types", () => config);

import { MemorySection } from "../memory-section";

describe("MemorySection", () => {
  beforeEach(() => {
    config.getConfig.mockReset();
    config.getConfig.mockResolvedValue({ memory: { backend: "basic" } });
  });

  it("patches private_memory when the toggle is flipped", async () => {
    const onChange = vi.fn();
    render(<MemorySection privateMemory={false} onChange={onChange} />);

    // The section renders two buttons: the description's help tip (which carries
    // aria-expanded) and the toggle itself.
    const toggle = screen
      .getAllByRole("button")
      .find((b) => !b.hasAttribute("aria-expanded"))!;
    fireEvent.click(toggle);

    expect(onChange).toHaveBeenCalledWith({ private_memory: true });
  });

  it("describes the consequences for the backend the server actually runs", async () => {
    config.getConfig.mockResolvedValue({ memory: { backend: "pkm" } });
    render(<MemorySection privateMemory onChange={vi.fn()} />);

    expect(await screen.findByText(/memory_remember still works/)).toBeInTheDocument();
    expect(screen.getByText(/never consolidated/)).toBeInTheDocument();
    // The Basic-only copy must not leak into a PKM install.
    expect(screen.queryByText(/store_user_memory/)).not.toBeInTheDocument();
  });

  it("falls back to backend-agnostic copy when the config is unreadable", async () => {
    config.getConfig.mockRejectedValue(new Error("403"));
    render(<MemorySection privateMemory={false} onChange={vi.fn()} />);

    await waitFor(() => expect(config.getConfig).toHaveBeenCalled());
    expect(
      screen.getByText(/Nothing this agent learns is written where your other agents can read it\./),
    ).toBeInTheDocument();
  });
});
