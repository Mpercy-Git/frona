"use client";

import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  useRef,
  createElement,
} from "react";
import { api, archiveChat as apiArchiveChat, unarchiveChat as apiUnarchiveChat, deleteChat as apiDeleteChat, archiveSpace as apiArchiveSpace, deleteSpace as apiDeleteSpace, deleteAgent as apiDeleteAgent, deleteTask as apiDeleteTask, getArchivedChats, getContacts, getTask } from "./api-client";
import type {
  SpaceWithChats,
  ChatResponse,
  TaskResponse,
  Agent,
  Contact,
} from "./types";
import { indexContactsById } from "./types";

type ActiveTab = "chat" | "tasks";

/// Internal shape. Boot fields are `T | undefined` until `refresh()` lands;
/// `useNavigation` narrows them and throws if called before AppGate marks
/// the data ready, so consumers get the [[NavigationContextValue]] view.
interface NavigationContextRaw {
  spaces: SpaceWithChats[] | undefined;
  standaloneChats: ChatResponse[] | undefined;
  tasks: TaskResponse[] | undefined;
  agents: Agent[] | undefined;
  contacts: Record<string, Contact> | undefined;
  archivedChats: ChatResponse[];
  showArchived: boolean;
  setShowArchived: (show: boolean) => void;
  activeTab: ActiveTab;
  setActiveTab: (tab: ActiveTab) => void;
  mobileNavOpen: boolean;
  setMobileNavOpen: (open: boolean) => void;
  mobileSubNavOpen: boolean;
  setMobileSubNavOpen: (open: boolean) => void;
  /// Throws on network unavailability — AppGate decides whether to retry.
  refresh: () => Promise<void>;
  addStandaloneChat: (chat: ChatResponse) => void;
  addChat: (chat: ChatResponse) => void;
  addChatById: (chatId: string) => void;
  updateChatTitle: (chatId: string, title: string) => void;
  updateAgent: (agentId: string, fields: Record<string, unknown>) => void;
  deleteAgent: (agentId: string) => Promise<void>;
  archiveChat: (chatId: string) => Promise<void>;
  unarchiveChat: (chatId: string) => Promise<void>;
  deleteChat: (chatId: string) => Promise<void>;
  archiveSpace: (spaceId: string) => Promise<void>;
  deleteSpace: (spaceId: string) => Promise<void>;
  deleteTask: (taskId: string) => Promise<void>;
  updateTaskInList: (taskId: string, fields: Partial<TaskResponse>) => void;
}

export type NavigationContextValue = Omit<
  NavigationContextRaw,
  "spaces" | "standaloneChats" | "tasks" | "agents" | "contacts"
> & {
  spaces: SpaceWithChats[];
  standaloneChats: ChatResponse[];
  tasks: TaskResponse[];
  agents: Agent[];
  contacts: Record<string, Contact>;
};

const NavigationContext = createContext<NavigationContextRaw | null>(null);

interface NavigationResponse {
  spaces: SpaceWithChats[];
  standalone_chats: ChatResponse[];
}

export function NavigationProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const sortAgents = (list: Agent[]) =>
    [...list].sort((a, b) =>
      a.handle === "system" ? -1 : b.handle === "system" ? 1 : a.name.localeCompare(b.name),
    );

  const [spaces, setSpaces] = useState<SpaceWithChats[] | undefined>(undefined);
  const [standaloneChats, setStandaloneChats] = useState<ChatResponse[] | undefined>(undefined);
  const [tasks, setTasks] = useState<TaskResponse[] | undefined>(undefined);
  const [agents, setAgents] = useState<Agent[] | undefined>(undefined);
  const [contacts, setContacts] = useState<Record<string, Contact> | undefined>(undefined);
  const [archivedChats, setArchivedChats] = useState<ChatResponse[]>([]);
  const [showArchived, setShowArchived] = useState(false);
  const [activeTab, setActiveTab] = useState<ActiveTab>("chat");
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [mobileSubNavOpen, setMobileSubNavOpen] = useState(false);

  /// State is set atomically on success — UI sees the previous snapshot
  /// until the new one lands.
  const refresh = useCallback(async () => {
    const [nav, tasksData, agentsData, contactsData] = await Promise.all([
      api.get<NavigationResponse>("/api/navigation"),
      api.get<TaskResponse[]>("/api/tasks"),
      api.get<Agent[]>("/api/agents"),
      getContacts(),
    ]);
    setSpaces(nav.spaces);
    setStandaloneChats(nav.standalone_chats);
    setTasks(tasksData);
    setAgents(sortAgents(agentsData));
    setContacts(indexContactsById(contactsData));
  }, []);

  // `prev ? ... : prev` guards exist only to satisfy the type system; the
  // mutators are only reachable below AppGate, which guarantees boot data.
  // Deduped: the chat this client just created also arrives as an SSE
  // `entity_updated` event, and either path can land first.
  const addStandaloneChat = useCallback((chat: ChatResponse) => {
    setStandaloneChats((prev) => {
      if (!prev || prev.some((c) => c.id === chat.id)) return prev;
      return [chat, ...prev];
    });
  }, []);

  /// Slot a chat into the sidebar wherever it belongs — its space, or the
  /// standalone list. Idempotent: a chat we already show is left alone, so
  /// this is safe to call for chats this client created itself.
  const addChat = useCallback((chat: ChatResponse) => {
    // Task-execution chats and archived ones aren't sidebar entries — mirrors
    // what /api/navigation returns.
    if (chat.task_id || chat.archived_at) return;

    const spaceId = chat.space_id;
    if (spaceId) {
      setSpaces((prev) => {
        if (!prev) return prev;
        const space = prev.find((s) => s.id === spaceId);
        // A space we don't know about yet — the next refresh() brings both.
        if (!space || space.chats.some((c) => c.id === chat.id)) return prev;
        return prev.map((s) =>
          s.id === spaceId ? { ...s, chats: [chat, ...s.chats] } : s,
        );
      });
      return;
    }

    setStandaloneChats((prev) => {
      if (!prev || prev.some((c) => c.id === chat.id)) return prev;
      return [chat, ...prev];
    });
  }, []);

  /// A chat created server-side — an inbound call an agent answered, a
  /// message arriving on a channel — reaches this client only as an SSE
  /// `entity_updated` event carrying its id, so fetch the row and add it.
  /// Without this the sidebar learns about such chats only on a full reload.
  const inFlightChatFetches = useRef(new Set<string>());
  const addChatById = useCallback((chatId: string) => {
    if (inFlightChatFetches.current.has(chatId)) return;
    inFlightChatFetches.current.add(chatId);
    api
      .get<ChatResponse>(`/api/chats/${chatId}`)
      .then(addChat)
      .catch(() => {})
      .finally(() => inFlightChatFetches.current.delete(chatId));
  }, [addChat]);

  const updateAgent = useCallback((agentId: string, fields: Record<string, unknown>) => {
    setAgents((prev) => {
      if (!prev) return prev;
      const exists = prev.some((a) => a.id === agentId);
      if (exists) {
        return prev.map((a) => (a.id === agentId ? { ...a, ...fields } : a));
      }
      api.get<Agent>(`/api/agents/${agentId}`).then((agent) => {
        setAgents((curr) => {
          if (!curr) return curr;
          if (curr.some((a) => a.id === agentId)) return curr;
          return sortAgents([...curr, agent]);
        });
      }).catch(() => {});
      return prev;
    });
  }, []);

  const deleteAgentAction = useCallback(async (agentId: string) => {
    await apiDeleteAgent(agentId);
    setAgents((prev) => prev?.filter((a) => a.id !== agentId));
  }, []);

  const archiveChat = useCallback(async (chatId: string) => {
    await apiArchiveChat(chatId);
    setStandaloneChats((prev) => prev?.filter((c) => c.id !== chatId));
    setSpaces((prev) =>
      prev?.map((space) => ({
        ...space,
        chats: space.chats.filter((c) => c.id !== chatId),
      })),
    );
    const archived = await getArchivedChats();
    setArchivedChats(archived);
  }, []);

  const unarchiveChat = useCallback(async (chatId: string) => {
    await apiUnarchiveChat(chatId);
    setArchivedChats((prev) => prev.filter((c) => c.id !== chatId));
    await refresh();
  }, [refresh]);

  const deleteChatAction = useCallback(async (chatId: string) => {
    await apiDeleteChat(chatId);
    setStandaloneChats((prev) => prev?.filter((c) => c.id !== chatId));
    setSpaces((prev) =>
      prev?.map((space) => ({
        ...space,
        chats: space.chats.filter((c) => c.id !== chatId),
      })),
    );
    setArchivedChats((prev) => prev.filter((c) => c.id !== chatId));
  }, []);

  const archiveSpaceAction = useCallback(async (spaceId: string) => {
    await apiArchiveSpace(spaceId);
    setSpaces((prev) => prev?.filter((s) => s.id !== spaceId));
  }, []);

  const deleteSpaceAction = useCallback(async (spaceId: string) => {
    await apiDeleteSpace(spaceId);
    setSpaces((prev) => prev?.filter((s) => s.id !== spaceId));
  }, []);

  const deleteTaskAction = useCallback(async (taskId: string) => {
    const task = tasks?.find((t) => t.id === taskId);
    await apiDeleteTask(taskId);
    setTasks((prev) => prev?.filter((t) => t.id !== taskId));
    if (task?.chat_id) {
      const chatId = task.chat_id;
      setStandaloneChats((prev) => prev?.filter((c) => c.id !== chatId));
      setSpaces((prev) =>
        prev?.map((space) => ({
          ...space,
          chats: space.chats.filter((c) => c.id !== chatId),
        })),
      );
    }
  }, [tasks]);

  const updateTaskInList = useCallback((taskId: string, fields: Partial<TaskResponse>) => {
    setTasks((prev) => {
      if (!prev) return prev;
      const idx = prev.findIndex((t) => t.id === taskId);
      if (idx !== -1) {
        const updated = [...prev];
        updated[idx] = { ...updated[idx], ...fields };
        return updated;
      }
      return prev;
    });
    const status = fields.status ?? "pending";
    if (status === "pending" || status === "inprogress") {
      getTask(taskId)
        .then((task) => {
          setTasks((prev) => {
            if (!prev) return prev;
            if (prev.some((t) => t.id === task.id)) return prev;
            return [task, ...prev];
          });
        })
        .catch(() => {});
    }
  }, []);

  useEffect(() => {
    if (showArchived) {
      getArchivedChats().then(setArchivedChats).catch(() => {});
    }
  }, [showArchived]);

  const updateChatTitle = useCallback((chatId: string, title: string) => {
    setStandaloneChats((prev) =>
      prev?.map((c) => (c.id === chatId ? { ...c, title } : c)),
    );
    setSpaces((prev) =>
      prev?.map((space) => ({
        ...space,
        chats: space.chats.map((c) =>
          c.id === chatId ? { ...c, title } : c,
        ),
      })),
    );
  }, []);

  return createElement(
    NavigationContext.Provider,
    {
      value: {
        spaces,
        standaloneChats,
        tasks,
        agents,
        contacts,
        archivedChats,
        showArchived,
        setShowArchived,
        activeTab,
        setActiveTab,
        mobileNavOpen,
        setMobileNavOpen,
        mobileSubNavOpen,
        setMobileSubNavOpen,
        refresh,
        addStandaloneChat,
        addChat,
        addChatById,
        updateChatTitle,
        updateAgent,
        deleteAgent: deleteAgentAction,
        archiveChat,
        unarchiveChat,
        deleteChat: deleteChatAction,
        archiveSpace: archiveSpaceAction,
        deleteSpace: deleteSpaceAction,
        deleteTask: deleteTaskAction,
        updateTaskInList,
      },
    },
    children,
  );
}

export function useNavigation(): NavigationContextValue {
  const ctx = useContext(NavigationContext);
  if (!ctx)
    throw new Error("useNavigation must be used within NavigationProvider");
  if (
    ctx.spaces === undefined ||
    ctx.standaloneChats === undefined ||
    ctx.tasks === undefined ||
    ctx.agents === undefined ||
    ctx.contacts === undefined
  ) {
    throw new Error(
      "useNavigation called before AppGate finished loading. Components below AppGate can rely on boot data being present; if you need the loose shape (e.g. inside AppGate itself), use useNavigationRaw.",
    );
  }
  return ctx as NavigationContextValue;
}

/// Only AppGate needs this — it gates rendering on whether data has loaded.
export function useNavigationRaw(): NavigationContextRaw {
  const ctx = useContext(NavigationContext);
  if (!ctx)
    throw new Error("useNavigationRaw must be used within NavigationProvider");
  return ctx;
}

export function useSystemAgent(): Agent {
  const { agents } = useNavigation();
  const found = agents.find((a) => a.handle === "system");
  if (!found) {
    throw new Error(
      "Invariant violated: 'system' agent missing. Every user is supposed to have one cloned at signup.",
    );
  }
  return found;
}

export function neighborRoute(
  items: { id: string }[],
  deletedId: string,
  urlFn: (id: string) => string,
): string | null {
  const idx = items.findIndex((item) => item.id === deletedId);
  if (idx === -1) return null;
  const neighbor = items[idx + 1] ?? items[idx - 1];
  return neighbor ? urlFn(neighbor.id) : null;
}
