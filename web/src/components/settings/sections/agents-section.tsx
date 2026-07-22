"use client";

import { useRouter } from "next/navigation";
import { UserGroupIcon, Cog6ToothIcon, TrashIcon } from "@heroicons/react/24/outline";
import { useNavigation } from "@/lib/navigation-context";
import { agentDisplayName, type Agent } from "@/lib/types";
import { SectionHeader } from "../field";

function AgentIcon({ agent }: { agent: Agent }) {
  if (agent.avatar_url) {
    return (
      <img
        src={agent.avatar_url}
        alt={agent.name}
        className="h-9 w-9 shrink-0 rounded-full object-cover"
      />
    );
  }
  const name = agentDisplayName(agent.id, agent.name);
  return (
    <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-sm font-medium bg-surface-tertiary text-text-secondary">
      {name.charAt(0).toUpperCase()}
    </div>
  );
}

export function AgentsSection() {
  const router = useRouter();
  const { agents, deleteAgent } = useNavigation();
  const list = agents ?? [];

  const handleDelete = async (agent: Agent) => {
    if (!confirm(`Delete agent "${agentDisplayName(agent.id, agent.name)}"?`)) return;
    await deleteAgent(agent.id);
  };

  return (
    <div className="space-y-6">
      <SectionHeader title="Agents" description="Configure and manage your agents" icon={UserGroupIcon} />

      <div className="rounded-xl border border-border bg-surface-secondary overflow-hidden divide-y divide-border">
        {list.map((agent) => (
          <div
            key={agent.id}
            className="group flex items-center gap-3 px-4 py-3 hover:bg-surface-tertiary transition"
          >
            <AgentIcon agent={agent} />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate text-sm font-medium text-text-primary">
                  {agentDisplayName(agent.id, agent.name)}
                </span>
                {agent.is_shared && (
                  <span className="shrink-0 rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-tertiary">
                    {agent.shared_by ? `Shared by @${agent.shared_by}` : "Shared"}
                  </span>
                )}
                {!agent.enabled && (
                  <span className="shrink-0 rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-tertiary">
                    Disabled
                  </span>
                )}
              </div>
              {agent.description && (
                <p className="truncate text-xs text-text-tertiary mt-0.5">{agent.description}</p>
              )}
            </div>

            <button
              type="button"
              onClick={() => router.push(`/agents?id=${agent.id}`)}
              className="inline-flex items-center gap-1 rounded-lg border border-border px-2.5 py-1.5 text-xs text-text-secondary hover:bg-surface hover:text-text-primary transition"
              title="Configure"
            >
              <Cog6ToothIcon className="h-4 w-4" />
              Configure
            </button>
            {!agent.is_shared && (
              <button
                type="button"
                onClick={() => handleDelete(agent)}
                className="p-1.5 rounded-lg text-text-tertiary hover:text-error-text hover:bg-surface transition"
                title="Delete"
              >
                <TrashIcon className="h-4 w-4" />
              </button>
            )}
          </div>
        ))}

        {list.length === 0 && (
          <p className="px-4 py-10 text-center text-sm text-text-tertiary">
            No agents yet. Ask the assistant to &ldquo;create a new agent&rdquo; to get started.
          </p>
        )}
      </div>
    </div>
  );
}
