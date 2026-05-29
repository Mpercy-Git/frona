"use client";

import { useState } from "react";
import { useRouter, useSearchParams, usePathname } from "next/navigation";
import {
  FolderPlusIcon,
  FolderIcon,
  PlusIcon,
  ArchiveBoxIcon,
  ChevronDownIcon,
  ChevronRightIcon,
} from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import { useNavigation, neighborRoute } from "@/lib/navigation-context";
import { useSession } from "@/lib/session-context";
import { ChatActions } from "./chat-actions";
import { DeleteConfirmDialog } from "./delete-confirm-dialog";
import { TaskItem } from "./task-item";
import type { SpaceResponse } from "@/lib/types";

export function ChatsTab() {
  const {
    spaces,
    standaloneChats,
    tasks,
    archivedChats,
    showArchived,
    setShowArchived,
    refresh,
    archiveChat,
    unarchiveChat,
    deleteChat,
  } = useNavigation();
  const { activeChatId, activeChat, setActiveChat } = useSession();
  const selectedChatId = activeChatId ?? activeChat?.id ?? null;
  const router = useRouter();
  const searchParams = useSearchParams();
  const pathname = usePathname();
  const activeSpaceId = pathname === "/space"
    ? searchParams.get("id")
    : activeChat?.space_id ?? null;
  const taskChats = tasks.filter((task) => task.chat_id && !task.space_id);
  const [creatingSpace, setCreatingSpace] = useState(false);
  const [spaceName, setSpaceName] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  const handleNewChat = () => {
    setActiveChat(null);
    router.push("/chat");
  };

  const handleCreateSpace = async (e: React.FormEvent) => {
    e.preventDefault();
    const name = spaceName.trim();
    if (!name) return;
    await api.post<SpaceResponse>("/api/spaces", { name });
    setSpaceName("");
    setCreatingSpace(false);
    refresh();
  };

  const handleArchive = async (chatId: string) => {
    await archiveChat(chatId);
    if (selectedChatId === chatId) {
      router.push("/chat");
    }
  };

  const handleUnarchive = async (chatId: string) => {
    await unarchiveChat(chatId);
  };

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    const wasActive = selectedChatId === deleteTarget;
    const next =
      neighborRoute(standaloneChats, deleteTarget, (id) => `/chat?id=${id}`) ??
      neighborRoute(archivedChats, deleteTarget, (id) => `/chat?id=${id}`);
    await deleteChat(deleteTarget);
    setDeleteTarget(null);
    if (wasActive) {
      router.push(next ?? "/chat");
    }
  };

  return (
    <div className="space-y-1 p-2">
      <div className="flex items-center justify-between px-2 pb-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-text-tertiary">
          Spaces
        </span>
        <button
          onClick={() => setCreatingSpace((v) => !v)}
          className="rounded p-0.5 text-text-tertiary hover:text-text-primary transition"
          title="New space"
        >
          <FolderPlusIcon className="h-3.5 w-3.5" />
        </button>
      </div>

      {creatingSpace && (
        <form onSubmit={handleCreateSpace} className="px-2 pb-1">
          <input
            autoFocus
            value={spaceName}
            onChange={(e) => setSpaceName(e.target.value)}
            onBlur={() => {
              if (!spaceName.trim()) setCreatingSpace(false);
            }}
            placeholder="Space name..."
            className="w-full rounded-lg border border-border bg-surface px-2 py-1 text-sm text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-text-secondary"
          />
        </form>
      )}

      {spaces.map((space) => (
        <button
          key={space.id}
          onClick={() => router.push(`/space?id=${space.id}`)}
          className={`flex w-full items-center gap-1 rounded-lg px-3 py-2 text-sm font-medium transition ${
            activeSpaceId === space.id
              ? "bg-surface-tertiary text-text-primary"
              : "text-text-primary hover:bg-surface-secondary"
          }`}
        >
          <FolderIcon className="h-4 w-4 shrink-0 text-text-tertiary" />
          <span className="truncate">{space.name}</span>
          <span className="ml-auto text-[10px] text-text-tertiary">
            {space.chats.length}
          </span>
        </button>
      ))}

      {standaloneChats.length > 0 && (
        <div className="pt-2">
          <div className="flex items-center justify-between px-2 pb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-text-tertiary">
              Chats
            </span>
            <button
              onClick={handleNewChat}
              className="rounded p-0.5 text-text-tertiary hover:text-text-primary transition"
              title="New chat"
            >
              <PlusIcon className="h-3.5 w-3.5" />
            </button>
          </div>
          {standaloneChats.map((chat) => (
            <div
              key={chat.id}
              className={`group flex items-center rounded-lg pr-1 transition ${
                selectedChatId === chat.id
                  ? "bg-surface-tertiary text-text-primary"
                  : "text-text-secondary hover:bg-surface-secondary"
              }`}
            >
              <button
                onClick={() => router.push(`/chat?id=${chat.id}`)}
                className="flex-1 min-w-0 px-3 py-2 text-left text-sm truncate"
              >
                {chat.title ?? "New chat"}
              </button>
              <ChatActions
                isArchived={false}
                onArchive={() => handleArchive(chat.id)}
                onUnarchive={() => {}}
                onDelete={() => setDeleteTarget(chat.id)}
              />
            </div>
          ))}
        </div>
      )}

      {taskChats.length > 0 && (
        <div className="pt-2">
          <div className="px-2 pb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-text-tertiary">
              Task chats
            </span>
          </div>
          {taskChats.map((task) => (
            <TaskItem key={task.id} task={task} />
          ))}
        </div>
      )}

      <div className="pt-2">
        <button
          onClick={() => setShowArchived(!showArchived)}
          className="flex w-full items-center gap-1.5 px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-text-tertiary hover:text-text-secondary transition"
        >
          <ArchiveBoxIcon className="h-3 w-3" />
          Archived
          {showArchived ? (
            <ChevronDownIcon className="ml-auto h-3 w-3" />
          ) : (
            <ChevronRightIcon className="ml-auto h-3 w-3" />
          )}
        </button>
        {showArchived &&
          archivedChats.map((chat) => (
            <div
              key={chat.id}
              className={`group flex items-center rounded-lg pr-1 transition ${
                selectedChatId === chat.id
                  ? "bg-surface-tertiary text-text-primary"
                  : "text-text-secondary hover:bg-surface-secondary"
              }`}
            >
              <button
                onClick={() => router.push(`/chat?id=${chat.id}`)}
                className="flex-1 min-w-0 px-3 py-2 text-left text-sm truncate"
              >
                {chat.title ?? "New chat"}
              </button>
              <ChatActions
                isArchived
                onArchive={() => {}}
                onUnarchive={() => handleUnarchive(chat.id)}
                onDelete={() => setDeleteTarget(chat.id)}
              />
            </div>
          ))}
        {showArchived && archivedChats.length === 0 && (
          <p className="px-3 py-2 text-xs text-text-tertiary">
            No archived chats
          </p>
        )}
      </div>

      {spaces.length === 0 && standaloneChats.length === 0 && taskChats.length === 0 && !creatingSpace && (
        <p className="px-2 py-4 text-center text-xs text-text-tertiary">
          No chats yet. Start a new conversation!
        </p>
      )}

      <DeleteConfirmDialog
        open={deleteTarget !== null}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={handleDeleteConfirm}
      />
    </div>
  );
}
