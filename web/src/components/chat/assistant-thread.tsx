"use client";

import { useCallback, useMemo, useRef } from "react";
import { ThreadPrimitive } from "@assistant-ui/react";
import { FronaUserMessage } from "./frona-user-message";
import { FronaAssistantMessage } from "./frona-assistant-message";
import { FronaComposer } from "./frona-composer";
import { ExternalToolDrawer, CollapsedToolTab, useToolWizard } from "./external-tool-drawer";
import { WizardAnswersContext } from "@/lib/wizard-answers-context";
import { usePendingTools } from "@/lib/pending-tools-context";

export function AssistantThread() {
  const wizard = useToolWizard();
  const wizardSetCollapsed = wizard.setCollapsed;
  const lastScrollTop = useRef(0);
  const updating = useRef(false);

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
      if (updating.current) return;

      const el = e.currentTarget;
      const { scrollTop, scrollHeight, clientHeight } = el;
      const delta = scrollTop - lastScrollTop.current;
      lastScrollTop.current = scrollTop;

      const isNearBottom = scrollHeight - scrollTop - clientHeight < 80;

      if (delta < -10 && !isNearBottom && wizard.submitted) {
        setCollapsed(true);
      } else if (isNearBottom) {
        setCollapsed(false);
      }
    },
    [setCollapsed, wizard.submitted],
  );

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
