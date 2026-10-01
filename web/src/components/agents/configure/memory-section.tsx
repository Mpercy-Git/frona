"use client";

import { useEffect, useState } from "react";
import { SectionHeader, SectionPanel, Toggle } from "@/components/settings/field";
import { LockClosedIcon } from "@heroicons/react/24/outline";
import { getConfig } from "@/lib/config-types";
import type { MemoryBackend } from "@/lib/config-types";

/**
 * What "private memory" costs this agent, in the terms of whichever backend the
 * server actually runs. The two differ in what an agent keeps: Basic still has its
 * own agent-scoped notes, PKM keeps writing short memories but only for itself.
 */
const CONSEQUENCES: Record<MemoryBackend, string[]> = {
  basic: [
    "It loses the store_user_memory tool — nothing it learns is written to the memory your other agents share.",
    "Its own store_agent_memory notes work exactly as before. They were never visible to another agent.",
    "Its chats are left out of space summaries, so nothing reaches another agent that way either.",
    "It still reads your shared user memory and space context. Private means it does not write there, not that it is cut off.",
  ],
  pkm: [
    "memory_remember still works, but what it writes comes back only for this agent.",
    "Its conversations are never consolidated, so nothing from them becomes a knowledge page in your vault.",
    "It still searches, cites and reads the knowledge base like any other agent. Private means it does not add to it.",
  ],
};

const GENERIC = [
  "Nothing this agent learns is written where your other agents can read it.",
  "It still reads the memory you already have. Private means it does not add to it.",
];

export function MemorySection({
  privateMemory,
  onChange,
}: {
  privateMemory: boolean;
  onChange: (fields: { private_memory: boolean }) => void;
}) {
  const [backend, setBackend] = useState<MemoryBackend | null>(null);

  useEffect(() => {
    getConfig()
      .then((c) => setBackend(c.memory.backend))
      .catch(() => {});
  }, []);

  const points = backend ? CONSEQUENCES[backend] : GENERIC;

  return (
    <div>
      <SectionHeader
        title="Memory"
        description="Whether what this agent learns is shared with your other agents"
        icon={LockClosedIcon}
      />
      <SectionPanel>
        <Toggle
          label="Private memory"
          description="Keep everything this agent remembers to itself."
          value={privateMemory}
          onChange={(v) => onChange({ private_memory: v })}
        />
        <ul className="space-y-2 text-sm text-text-tertiary">
          {points.map((point) => (
            <li key={point} className="flex gap-2">
              <span aria-hidden className="text-text-tertiary">
                •
              </span>
              <span>{point}</span>
            </li>
          ))}
        </ul>
        {privateMemory && (
          <p className="text-xs text-text-tertiary">
            Turning this off again does not backfill: anything remembered while it was on
            stays private to this agent.
          </p>
        )}
      </SectionPanel>
    </div>
  );
}
