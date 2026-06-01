"use client";

import { useRef, useState, useEffect, useCallback } from "react";
import { ComposerPrimitive, ThreadPrimitive, AttachmentPrimitive, useComposerRuntime } from "@assistant-ui/react";
import { useThreadIsRunning } from "@assistant-ui/core/react";
import { PaperAirplaneIcon, StopIcon, PlusIcon, XMarkIcon } from "@heroicons/react/24/solid";
import { ArrowUpTrayIcon, CloudIcon, FolderOpenIcon } from "@heroicons/react/24/outline";
import { FileBrowserModal } from "@/components/chat/file-browser-modal";
import { registerBackendAttachment, getBackendAttachment } from "@/lib/use-chat-runtime";
import { useContext } from "react";
import { usePendingTools } from "@/lib/pending-tools-context";
import { ChatContext } from "@/lib/chat-context";
import { api } from "@/lib/api-client";
import type { ToolWizardState } from "./external-tool-drawer";
import type { Attachment } from "@/lib/types";

function ComposerAttachmentBadge() {
  return (
    <AttachmentPrimitive.Root className="inline-flex items-center gap-1 rounded-md bg-surface-tertiary px-2 py-1 text-xs text-text-secondary">
      <span className="max-w-[200px] truncate">
        <AttachmentPrimitive.Name />
      </span>
      <AttachmentPrimitive.Remove asChild>
        <button type="button" className="ml-0.5 hover:text-text-primary">
          <XMarkIcon className="h-3 w-3" />
        </button>
      </AttachmentPrimitive.Remove>
    </AttachmentPrimitive.Root>
  );
}

export function FronaComposer({
  placeholder = "Send a message...",
  onSend,
  wizard,
}: {
  placeholder?: string;
  /** Called on form submit for custom send handling (e.g. HomeComposer navigation). */
  onSend?: (content: string, attachments?: Attachment[]) => void;
  wizard?: ToolWizardState;
}) {
  const composerRuntime = useComposerRuntime();
  const [menuOpen, setMenuOpen] = useState(false);
  const [browseOpen, setBrowseOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);

  const threadRunning = useThreadIsRunning();
  const pendingTools = usePendingTools();
  const chatCtx = useContext(ChatContext);
  const currentIndex = wizard?.currentIndex ?? 0;
  const safeIndex = Math.min(currentIndex, Math.max(0, pendingTools.length - 1));
  const currentPendingTool = pendingTools.length > 0 ? pendingTools[safeIndex] : undefined;

  const handleWizardAnswer = useCallback(() => {
    if (!currentPendingTool || !wizard) return;
    const text = composerRuntime.getState().text.trim();
    if (!text) return;
    composerRuntime.setText("");

    // Question: typed text is a valid freeform answer (success)
    // Other tools: typed text is a reason for declining (fail)
    const isQuestion = currentPendingTool.tool_data?.type === "Question";
    const action: "success" | "fail" = isQuestion ? "success" : "fail";

    const nextAnswers = new Map(wizard.answers);
    nextAnswers.set(currentPendingTool.id, { response: text, action });
    wizard.setAnswers(nextAnswers);

    // If all tools now have answers, auto-submit
    const allNowAnswered = pendingTools.every((te) => nextAnswers.has(te.id));
    if (allNowAnswered && chatCtx?.chatId) {
      const callbacks = pendingTools
        .map((te) => nextAnswers.get(te.id)?.callback)
        .filter((cb): cb is () => Promise<void> => !!cb);
      Promise.all(callbacks.map((cb) => cb()));
      const resolutions = pendingTools
        .filter((te) => !nextAnswers.get(te.id)?.callback)
        .map((te) => {
          const ans = nextAnswers.get(te.id);
          return {
            tool_call_id: te.id,
            response: ans?.response ?? "User declined to answer",
            action: ans?.action ?? "fail",
          };
        });
      wizard.setSubmitted(true);
      if (resolutions.length > 0) {
        api.post(`/api/chats/${chatCtx.chatId}/tool-calls/resolve`, { resolutions });
      }
    } else if (safeIndex < pendingTools.length - 1) {
      wizard.setCurrentIndex(safeIndex + 1);
    }
  }, [currentPendingTool, wizard, composerRuntime, pendingTools, safeIndex, chatCtx?.chatId]);

  const activePlaceholder = currentPendingTool
    ? "Type your answer or click an option above..."
    : placeholder;

  useEffect(() => {
    if (!menuOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (
        menuRef.current && !menuRef.current.contains(e.target as Node) &&
        menuButtonRef.current && !menuButtonRef.current.contains(e.target as Node)
      ) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [menuOpen]);

  const handleBrowseSelect = useCallback(
    (files: Attachment[]) => {
      for (const file of files) {
        registerBackendAttachment(file.path, file);
        composerRuntime.addAttachment({
          id: file.path,
          type: "file",
          name: file.filename,
          contentType: file.content_type,
          content: [{ type: "text", text: `[file: ${file.filename}]` }],
        });
      }
    },
    [composerRuntime],
  );

  return (
    <div>
      <ComposerPrimitive.Root
        className={currentPendingTool && !wizard?.submitted
          ? "bg-transparent p-4"
          : "rounded-2xl border border-border bg-surface-secondary p-4 focus-within:border-accent transition-colors"
        }
        {...(currentPendingTool ? { onSubmit: handleWizardAnswer } : onSend ? { onSubmit: () => {
          const state = composerRuntime.getState();
          const text = state.text.trim();
          if (!text && !state.attachments.length) return;
          const backendAttachments = state.attachments
            .map((a) => getBackendAttachment(a.id))
            .filter((a): a is Attachment => a != null);
          onSend(text, backendAttachments.length > 0 ? backendAttachments : undefined);
        }} : {})}
      >
        <div className="flex flex-wrap gap-1.5 empty:hidden">
          <ComposerPrimitive.Attachments
            components={{ Attachment: ComposerAttachmentBadge }}
          />
        </div>
        <ComposerPrimitive.Input
          autoFocus
          placeholder={activePlaceholder}
          rows={1}
          className="w-full bg-transparent text-sm leading-5 text-text-primary placeholder:text-text-tertiary focus:outline-none resize-none max-h-[200px] overflow-y-auto"
        />
        <div className="flex items-center justify-between pt-2">
          <div className="relative">
            <button
              ref={menuButtonRef}
              type="button"
              onClick={() => setMenuOpen((v) => !v)}
              className="rounded-lg p-1 text-text-tertiary hover:text-text-secondary hover:bg-surface-tertiary transition"
              title="Attach"
            >
              <PlusIcon className="h-5 w-5" />
            </button>
            {menuOpen && (
              <div
                ref={menuRef}
                className="absolute bottom-full left-0 mb-1 w-max rounded-lg border border-border bg-surface-secondary py-1 shadow-lg"
              >
                <ComposerPrimitive.AddAttachment asChild multiple>
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 px-3 py-2 text-sm text-text-secondary hover:bg-surface-tertiary transition-colors"
                    onClick={() => setMenuOpen(false)}
                  >
                    <ArrowUpTrayIcon className="h-4 w-4" />
                    Upload files
                  </button>
                </ComposerPrimitive.AddAttachment>
                <button
                  type="button"
                  className="flex w-full items-center gap-2 px-3 py-2 text-sm text-text-secondary hover:bg-surface-tertiary transition-colors"
                  onClick={() => {
                    setBrowseOpen(true);
                    setMenuOpen(false);
                  }}
                >
                  <FolderOpenIcon className="h-4 w-4" />
                  Browse files
                </button>
                <button
                  type="button"
                  className="flex w-full items-center gap-2 px-3 py-2 text-sm text-text-tertiary cursor-default"
                  onClick={() => setMenuOpen(false)}
                >
                  <CloudIcon className="h-4 w-4" />
                  Connect to Google Drive
                </button>
              </div>
            )}
          </div>
          <div className="flex items-center gap-1">
            {wizard?.submitted && threadRunning ? (
              <ComposerPrimitive.Cancel asChild>
                <button
                  type="button"
                  className="shrink-0 rounded-lg p-1.5 text-text-secondary hover:text-text-primary transition"
                >
                  <StopIcon className="h-5 w-5" />
                </button>
              </ComposerPrimitive.Cancel>
            ) : (
            <>
            <ThreadPrimitive.If running>
              <ComposerPrimitive.Cancel asChild>
                <button
                  type="button"
                  className="shrink-0 rounded-lg p-1.5 text-text-secondary hover:text-text-primary transition"
                >
                  <StopIcon className="h-5 w-5" />
                </button>
              </ComposerPrimitive.Cancel>
            </ThreadPrimitive.If>
            <ThreadPrimitive.If running={false}>
              {onSend || currentPendingTool ? (
                <button
                  type="submit"
                  className="shrink-0 rounded-lg p-1.5 text-text-secondary hover:text-text-primary transition"
                >
                  <PaperAirplaneIcon className="h-5 w-5" />
                </button>
              ) : (
                <ComposerPrimitive.Send asChild>
                  <button
                    type="button"
                    className="shrink-0 rounded-lg p-1.5 text-text-secondary hover:text-text-primary transition"
                  >
                    <PaperAirplaneIcon className="h-5 w-5" />
                  </button>
                </ComposerPrimitive.Send>
              )}
            </ThreadPrimitive.If>
            </>
            )}
          </div>
        </div>
      </ComposerPrimitive.Root>
      <FileBrowserModal
        open={browseOpen}
        onClose={() => setBrowseOpen(false)}
        onSelect={handleBrowseSelect}
      />
    </div>
  );
}
