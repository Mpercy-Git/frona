"use client";

import { SectionHeader, SectionPanel } from "@/components/settings/field";
import { ArrowUturnLeftIcon, DocumentTextIcon } from "@heroicons/react/24/outline";

interface InstructionsSectionProps {
  prompt: string;
  defaultPrompt: string;
  onPromptChange: (prompt: string) => void;
}

export function InstructionsSection({ prompt, defaultPrompt, onPromptChange }: InstructionsSectionProps) {
  // Saving a prompt identical to the default clears the override server-side,
  // so resetting is just putting the default text back in the box.
  const isDefault = prompt === defaultPrompt;

  return (
    <div className="flex flex-col h-full">
      <SectionHeader title="Prompt" description="System prompt sent at the beginning of every conversation" icon={DocumentTextIcon} />
      <SectionPanel className="flex-1 flex flex-col">
        <div className="flex items-start justify-between gap-3">
          <p className="text-sm text-text-tertiary">
            The system prompt defines how this agent behaves, what it knows, and how it responds.{" "}
            <a
              href="https://docs.anthropic.com/en/docs/build-with-claude/prompt-engineering/overview"
              target="_blank"
              rel="noopener noreferrer"
              className="text-accent hover:underline"
            >
              Learn more about prompt engineering
            </a>
          </p>
          <button
            type="button"
            onClick={() => onPromptChange(defaultPrompt)}
            disabled={isDefault || !defaultPrompt}
            title={
              isDefault
                ? "Already using the default prompt"
                : "Discard your changes and restore this agent's default prompt"
            }
            className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-text-secondary hover:bg-surface-tertiary disabled:opacity-50 disabled:hover:bg-transparent transition"
          >
            <ArrowUturnLeftIcon className="h-3.5 w-3.5" />
            Reset to default
          </button>
        </div>
        <textarea
          value={prompt}
          onChange={(e) => onPromptChange(e.target.value)}
          className="flex-1 w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary font-mono placeholder:text-text-tertiary focus:border-accent focus:outline-none resize-none"
        />
      </SectionPanel>
    </div>
  );
}
