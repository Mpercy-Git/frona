import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  getPkmStatus: vi.fn(),
  requestPkmReset: vi.fn(),
}));

vi.mock("@/lib/api-client", () => api);

import { UserMemorySection } from "../user-memory-section";

describe("UserMemorySection", () => {
  beforeEach(() => {
    api.getPkmStatus.mockReset();
    api.requestPkmReset.mockReset();
    api.getPkmStatus.mockResolvedValue({ available: true, reset: null });
    api.requestPkmReset.mockResolvedValue({ requestId: "reset-1", status: "pending" });
  });

  it("confirms the full scope and shows accepted reset as background work", async () => {
    render(<UserMemorySection />);
    const resetButton = await screen.findByRole("button", { name: "Reset memory" });
    await waitFor(() => expect(resetButton).toBeEnabled());
    fireEvent.click(resetButton);

    expect(screen.getByText("Reset your memory?")).toBeInTheDocument();
    expect(screen.getByText(/managed Memory directory/)).toBeInTheDocument();
    expect(screen.getByText(/short-term memories remain/)).toBeInTheDocument();
    expect(screen.getByText(/normal consolidation schedule/)).toBeInTheDocument();
    expect(screen.getByText(/cannot be undone/)).toBeInTheDocument();

    const confirm = screen.getAllByRole("button", { name: "Reset memory" })[1];
    fireEvent.click(confirm);

    await waitFor(() => expect(api.requestPkmReset).toHaveBeenCalledOnce());
    expect(await screen.findByText("Your memory reset is running in the background."))
      .toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset running" })).toBeDisabled();
  });

  it("shows a failed reset and allows the user to retry it", async () => {
    api.getPkmStatus.mockResolvedValue({
      available: true,
      reset: {
        requestId: "failed-reset",
        status: "failed",
        requestedAt: "2026-08-17T12:00:00Z",
        startedAt: "2026-08-17T12:00:01Z",
        error: "The managed directory could not be deleted",
      },
    });

    render(<UserMemorySection />);

    expect(await screen.findByText(/managed directory could not be deleted/)).toBeInTheDocument();
    const retry = screen.getByRole("button", { name: "Retry reset" });
    expect(retry).toBeEnabled();
    fireEvent.click(retry);
    expect(screen.getByText("Reset your memory?")).toBeInTheDocument();
  });
});
