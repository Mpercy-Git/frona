import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  getMemoryGraph: vi.fn(),
  searchMemory: vi.fn(),
}));

vi.mock("@/lib/api-client", () => api);
vi.mock("@/lib/use-mobile", () => ({ useMobile: () => false }));
vi.mock("next/navigation", () => ({
  usePathname: () => "/memory",
  useSearchParams: () => new URLSearchParams(),
}));
vi.mock("../memory-graph-canvas", () => ({ MemoryGraphCanvas: () => null }));
vi.mock("../memory-inspector", () => ({ MemoryInspector: () => null }));

import { MemoryPage } from "../memory-page";

describe("MemoryPage", () => {
  beforeEach(() => {
    api.getMemoryGraph.mockReset();
    api.searchMemory.mockReset();
  });

  it("shows an empty state after a new user's empty graph loads", async () => {
    api.getMemoryGraph.mockResolvedValue({
      revision: "empty",
      selfPath: null,
      nodes: [],
      edges: [],
      legend: [],
    });

    render(<MemoryPage />);

    expect(screen.getByText("Loading memory graph…")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "No memories yet" })).toBeInTheDocument();
    expect(screen.queryByText("Loading memory graph…")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Start a conversation" })).toHaveAttribute("href", "/");
  });
});
