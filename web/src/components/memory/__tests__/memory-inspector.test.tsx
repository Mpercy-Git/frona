import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ getMemoryPage: vi.fn() }));

vi.mock("@/lib/api-client", () => api);
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: { aside: ({ children, ...props }: React.ComponentProps<"aside">) => <aside {...props}>{children}</aside> },
}));
vi.mock("@/components/ui/code-block", () => ({ CodeBlock: () => null }));

import { MemoryInspector } from "../memory-inspector";

const page = (overrides: Record<string, unknown> = {}) => ({
  page: {
    id: "1",
    path: "people/me",
    origin: "internal",
    category: "concept",
    kinds: [],
    name: "Morgan",
    description: "The account owner.",
    body: "",
    related_playbooks: [],
    attributes: {},
    use_count: 0,
    aliases: [],
    rev: null,
    updated_at: "2026-01-01T00:00:00Z",
    rendered_at: "2026-01-01T00:00:00Z",
  },
  types: [],
  attributes: [],
  outgoingRelations: [],
  incomingRelations: [],
  memories: [],
  ...overrides,
});

const browsable = (memoryCount: number) => ({
  path: "people/me",
  name: "Morgan",
  description: "The account owner.",
  useCount: 0,
  memoryCount,
  origin: "internal" as const,
  category: "concept" as const,
  types: [],
  aliases: [],
});

const props = {
  open: true,
  mobile: false,
  selectedPath: "people/me",
  tab: "page" as const,
  searchMode: false,
  searchQuery: "",
  searchResults: [],
  browsablePages: [],
  searchLoading: false,
  onClose: vi.fn(),
  onSelect: vi.fn(),
  onTab: vi.fn(),
  onBackToSearch: vi.fn(),
};

describe("MemoryInspector", () => {
  beforeEach(() => {
    api.getMemoryPage.mockReset();
  });

  it("explains an unwritten page instead of rendering a blank panel", async () => {
    api.getMemoryPage.mockResolvedValue(page());

    render(<MemoryInspector {...props} />);

    expect(await screen.findByText("No page written yet")).toBeInTheDocument();
    expect(screen.getByText(/Nothing has been learned about Morgan yet/)).toBeInTheDocument();
  });

  it("points at the Memory tab when a page has memories but no summary", async () => {
    api.getMemoryPage.mockResolvedValue(page({
      memories: [{
        id: "m1",
        created_at: "2026-01-01T00:00:00Z",
        kind: "fact",
        episode: null,
        content: "Morgan lives in Leeds.",
        relations: [],
        disposition: "none",
        ended_at: null,
        comment: null,
        erroneous_at: null,
        evidence: [],
      }],
    }));

    render(<MemoryInspector {...props} />);

    expect(await screen.findByText(/1 memory for Morgan but has not written the summary/)).toBeInTheDocument();
  });

  it("says nothing has been learned when every browsable page is an empty outline", async () => {
    render(<MemoryInspector {...props} searchMode browsablePages={[browsable(0)]} />);

    expect(screen.getByText("Nothing has been learned yet")).toBeInTheDocument();
    expect(screen.getByText("No memories")).toBeInTheDocument();
  });

  it("keeps the browse list plain once a page has memories behind it", async () => {
    render(<MemoryInspector {...props} searchMode browsablePages={[browsable(3)]} />);

    expect(screen.queryByText("Nothing has been learned yet")).not.toBeInTheDocument();
    expect(screen.getByText("3 memories")).toBeInTheDocument();
  });
});
