"use client";

import { useCallback, useLayoutEffect, useMemo, useRef } from "react";
import { ThreadPrimitive } from "@assistant-ui/react";
import { FronaUserMessage } from "./frona-user-message";
import { FronaAssistantMessage } from "./frona-assistant-message";
import { FronaComposer } from "./frona-composer";
import { ExternalToolDrawer, CollapsedToolTab, useToolWizard } from "./external-tool-drawer";
import { WizardAnswersContext } from "@/lib/wizard-answers-context";
import { usePendingTools } from "@/lib/pending-tools-context";
import { useChatPagination } from "@/lib/chat-pagination-context";

export function AssistantThread() {
  const wizard = useToolWizard();
  const wizardSetCollapsed = wizard.setCollapsed;
  const lastScrollTop = useRef(0);
  const updating = useRef(false);
  const { hasMore, loadingMore, loadOlder } = useChatPagination();
  const viewportElRef = useRef<HTMLElement | null>(null);
  // Anchor on a real message DOM node so scroll position survives the
  // prepend regardless of async height changes (markdown, images, etc).
  const anchorRef = useRef<{ id: string; offset: number } | null>(null);

  const setCollapsed = useCallback(
    (v: boolean | ((prev: boolean) => boolean)) => {
      updating.current = true;
      wizardSetCollapsed(v);
      requestAnimationFrame(() => {
        updating.current = false;
      });
    },
    [wizardSetCollapsed],
  );

  const handleScroll = useCallback(
    (e: React.UIEvent<HTMLDivElement>) => {
      const el = e.currentTarget;
      viewportElRef.current = el;
      if (updating.current) return;

      const { scrollTop, scrollHeight, clientHeight } = el;
      const delta = scrollTop - lastScrollTop.current;
      lastScrollTop.current = scrollTop;

      const isNearBottom = scrollHeight - scrollTop - clientHeight < 80;

      if (delta < -10 && !isNearBottom && wizard.submitted) {
        setCollapsed(true);
      } else if (isNearBottom) {
        setCollapsed(false);
      }

      if (scrollTop < 200 && hasMore && !loadingMore && !anchorRef.current) {
        const firstMsg = el.querySelector<HTMLElement>("[data-message-id]");
        if (firstMsg) {
          const vTop = el.getBoundingClientRect().top;
          anchorRef.current = {
            id: firstMsg.dataset.messageId!,
            offset: firstMsg.getBoundingClientRect().top - vTop,
          };
        }
        loadOlder();
      }
    },
    [setCollapsed, wizard.submitted, hasMore, loadingMore, loadOlder],
  );

  useLayoutEffect(() => {
    if (loadingMore) return;
    const el = viewportElRef.current;
    const anchor = anchorRef.current;
    if (!el || !anchor) return;
    const target = el.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(anchor.id)}"]`,
    );
    if (target) {
      const vTop = el.getBoundingClientRect().top;
      const currentOffset = target.getBoundingClientRect().top - vTop;
      el.scrollTop += currentOffset - anchor.offset;
    }
    anchorRef.current = null;
  }, [loadingMore]);

  const safeWizard = useMemo(
    () => ({ ...wizard, setCollapsed }),
    [wizard, setCollapsed],
  );

  const pendingTools = usePendingTools();
  const hasPendingTools = pendingTools.length > 0 && !wizard.submitted;

  return (
    <WizardAnswersContext value={wizard.answers}>
    <ThreadPrimitive.Root className="flex flex-1 flex-col min-h-0">
      <ThreadPrimitive.Viewport className="flex-1 overflow-y-auto min-h-0" onScroll={handleScroll}>
        <ThreadPrimitive.If empty>
          <div />
        </ThreadPrimitive.If>
        <ThreadPrimitive.If empty={false}>
          <div className="mx-auto w-full max-w-3xl px-3 md:px-6 py-4 space-y-3">
            <ThreadPrimitive.Messages
            components={{
              UserMessage: FronaUserMessage,
              AssistantMessage: FronaAssistantMessage,
            }}
          />
          </div>
        </ThreadPrimitive.If>
      </ThreadPrimitive.Viewport>
      <ThreadPrimitive.ViewportFooter className="sticky bottom-0">
        <ThreadPrimitive.ScrollToBottom asChild>
          <button className={`absolute left-1/2 -translate-x-1/2 z-20 rounded-full border border-border bg-surface px-3 py-1 text-xs text-text-secondary shadow-sm hover:bg-surface-secondary transition disabled:hidden ${
            hasPendingTools && safeWizard.collapsed ? "-top-16" : "-top-10"
          }`}>
            Scroll to bottom
          </button>
        </ThreadPrimitive.ScrollToBottom>
        <div className="relative mx-auto w-full max-w-3xl px-3 md:px-6 pb-4">
          <div className="absolute inset-x-0 -top-7 z-0 flex justify-center px-3 md:px-6">
            <CollapsedToolTab wizard={safeWizard} />
          </div>
          <div className={`relative z-10 rounded-2xl transition-colors ${
            hasPendingTools
              ? "border border-border bg-surface-secondary focus-within:border-accent"
              : "has-[.tool-drawer]:border has-[.tool-drawer]:border-border has-[.tool-drawer]:bg-surface-secondary has-[.tool-drawer]:focus-within:border-accent focus-within:border-accent"
          }`}>
            <ExternalToolDrawer wizard={safeWizard} />
            <FronaComposer wizard={safeWizard} />
          </div>
        </div>
      </ThreadPrimitive.ViewportFooter>
    </ThreadPrimitive.Root>
    </WizardAnswersContext>
  );
}
