import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const nav = vi.hoisted(() => ({
  useNavigation: vi.fn(),
  neighborRoute: vi.fn(() => null),
}));
const session = vi.hoisted(() => ({ useSession: vi.fn() }));
const routing = vi.hoisted(() => ({
  useRouter: () => ({ push: vi.fn() }),
  useSearchParams: () => new URLSearchParams(),
  usePathname: () => "/chat",
}));
const activity = vi.hoisted(() => ({ byChatId: new Map<string, string>() }));

vi.mock("@/lib/navigation-context", () => nav);
vi.mock("@/lib/session-context", () => session);
vi.mock("next/navigation", () => routing);
vi.mock("@/lib/chat-activity-context", () => ({
  useActivityOf: (chatId: string | null | undefined) =>
    (chatId && activity.byChatId.get(chatId)) || "idle",
  useAggregateActivity: (chatIds: string[]) => {
    let waiting = false;
    for (const id of chatIds) {
      const state = activity.byChatId.get(id);
      if (state === "working") return "working";
      if (state === "waiting") waiting = true;
    }
    return waiting ? "waiting" : "idle";
  },
}));

import { ChatsTab } from "../chats-tab";

const chat = (id: string, title: string) => ({
  id,
  title,
  space_id: null,
  task_id: null,
  agent_id: "agent-1",
  archived_at: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  is_shared: false,
});

function setupNavigation(overrides: Record<string, unknown> = {}) {
  nav.useNavigation.mockReturnValue({
    spaces: [],
    standaloneChats: [chat("chat-1", "Roof quote"), chat("chat-2", "Shopping list")],
    archivedChats: [],
    showArchived: false,
    setShowArchived: vi.fn(),
    refresh: vi.fn(),
    archiveChat: vi.fn(),
    unarchiveChat: vi.fn(),
    deleteChat: vi.fn(),
    archiveSpace: vi.fn(),
    deleteSpace: vi.fn(),
    ...overrides,
  });
  session.useSession.mockReturnValue({
    activeChatId: null,
    activeChat: null,
    setActiveChat: vi.fn(),
  });
}

describe("ChatsTab activity markers", () => {
  beforeEach(() => {
    activity.byChatId.clear();
    setupNavigation();
  });

  it("marks only the chat the agent is working in", () => {
    activity.byChatId.set("chat-1", "working");
    render(<ChatsTab />);

    expect(screen.getByLabelText("Roof quote: agent is working")).toBeInTheDocument();
    expect(screen.queryByLabelText("Shopping list: agent is working")).not.toBeInTheDocument();
  });

  it("marks a chat parked on an answer differently from one being worked on", () => {
    activity.byChatId.set("chat-1", "waiting");
    render(<ChatsTab />);

    expect(screen.getByLabelText("Roof quote: waiting for your answer")).toBeInTheDocument();
    expect(screen.queryByLabelText("Roof quote: agent is working")).not.toBeInTheDocument();
  });

  it("leaves an idle list unmarked", () => {
    render(<ChatsTab />);

    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });

  it("marks archived chats too — a background turn can outlive archiving", () => {
    setupNavigation({
      standaloneChats: [],
      archivedChats: [chat("chat-9", "Old thread")],
      showArchived: true,
    });
    activity.byChatId.set("chat-9", "working");
    render(<ChatsTab />);

    expect(screen.getByLabelText("Old thread: agent is working")).toBeInTheDocument();
  });

  it("surfaces a space whose chats are working, since the panel never lists them", () => {
    setupNavigation({
      spaces: [
        {
          id: "space-1",
          name: "House",
          chat_count: 1,
          chats: [chat("chat-5", "Builder")],
        },
      ],
      standaloneChats: [],
    });
    activity.byChatId.set("chat-5", "working");
    render(<ChatsTab />);

    expect(screen.getByLabelText("House: agent is working in this space")).toBeInTheDocument();
  });
});
