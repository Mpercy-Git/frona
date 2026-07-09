"use client";

import { useState, useRef, useEffect } from "react";
import { useRouter } from "next/navigation";
import {
  EllipsisVerticalIcon,
  ArchiveBoxIcon,
  ArchiveBoxXMarkIcon,
  TrashIcon,
} from "@heroicons/react/24/outline";
import { useNavigation, neighborRoute } from "@/lib/navigation-context";
import { agentDisplayName } from "@/lib/types";
import type { RunningTotals } from "@/lib/chat-store";
import { DeleteConfirmDialog } from "@/components/nav/delete-confirm-dialog";
import { UsagePill } from "./usage-pill";

interface ChatHeaderProps {
  /** This slot's chat id (`undefined` for a pending/unsaved slot) — drives
   *  title, agent name, and menu actions. **Not** read from session because
   *  session.activeChat is global and lags behind navigation in some paths;
   *  each slot's header should reflect the slot's own chat. */
  chatId?: string;
  agentId: string;
  totals: RunningTotals;
  lastFallbackIndex: number;
  lastChatInputTokens: number;
  totalToolCalls: number;
}

export function ChatHeader({
  chatId,
  agentId,
  totals,
  lastFallbackIndex,
  lastChatInputTokens,
  totalToolCalls,
}: ChatHeaderProps) {
  const { agents, standaloneChats, archivedChats, archiveChat, unarchiveChat, deleteChat } = useNavigation();
  const router = useRouter();
  const [menuOpen, setMenuOpen] = useState(false);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function handleClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [menuOpen]);

  const chat =
    chatId == null
      ? undefined
      : standaloneChats.find((c) => c.id === chatId)
        ?? archivedChats.find((c) => c.id === chatId);
  const agent = agents.find((a) => a.id === agentId);
  const agentName = agentDisplayName(agentId, agent?.name);
  const isArchived = !!chat?.archived_at;

  const handleArchive = async () => {
    if (!chat) return;
    setMenuOpen(false);
    await archiveChat(chat.id);
    router.push("/chat");
  };

  const handleUnarchive = async () => {
    if (!chat) return;
    setMenuOpen(false);
    await unarchiveChat(chat.id);
  };

  const handleDelete = async () => {
    if (!chat) return;
    const next =
      neighborRoute(standaloneChats, chat.id, (id) => `/chat?id=${id}`) ??
      neighborRoute(archivedChats, chat.id, (id) => `/chat?id=${id}`);
    setShowDeleteDialog(false);
    await deleteChat(chat.id);
    router.push(next ?? "/chat");
  };

  return (
    <div className="flex items-center border-b border-border px-4 md:px-6 py-3">
      <div className="flex-1 min-w-0">
        <h2 className="text-base font-semibold text-text-primary">
          {chat?.title ?? "New chat"}
        </h2>
        <div className="flex items-center justify-between gap-3">
          <p className="text-sm text-text-tertiary">{agentName}</p>
          {chat && totals.calls > 0 && (
            <UsagePill
              totals={totals}
              fallbackIndex={lastFallbackIndex}
              currentInputTokens={lastChatInputTokens}
              contextWindow={agent?.model?.context_window}
              totalToolCalls={totalToolCalls}
            />
          )}
        </div>
      </div>

      {chat && (
        <div ref={menuRef} className="relative ml-2">
          <button
            onClick={() => setMenuOpen((v) => !v)}
            className="rounded p-1 text-text-tertiary hover:text-text-primary transition"
          >
            <EllipsisVerticalIcon className="h-6 w-6" />
          </button>
          {menuOpen && (
            <div className="absolute right-0 top-full z-50 mt-1 w-36 rounded-lg border border-border bg-surface shadow-lg py-1">
              {isArchived ? (
                <button
                  onClick={handleUnarchive}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-text-secondary hover:bg-surface-secondary transition"
                >
                  <ArchiveBoxXMarkIcon className="h-4 w-4" />
                  Unarchive
                </button>
              ) : (
                <button
                  onClick={handleArchive}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-text-secondary hover:bg-surface-secondary transition"
                >
                  <ArchiveBoxIcon className="h-4 w-4" />
                  Archive
                </button>
              )}
              <button
                onClick={() => {
                  setMenuOpen(false);
                  setShowDeleteDialog(true);
                }}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-text-secondary hover:bg-surface-secondary transition"
              >
                <TrashIcon className="h-4 w-4" />
                Delete
              </button>
            </div>
          )}
        </div>
      )}
      <DeleteConfirmDialog
        open={showDeleteDialog}
        onCancel={() => setShowDeleteDialog(false)}
        onConfirm={handleDelete}
      />
    </div>
  );
}
