"use client";

import { useState, useRef, useEffect } from "react";
import { useRouter } from "next/navigation";
import {
  UserGroupIcon,
  Cog6ToothIcon,
  TrashIcon,
} from "@heroicons/react/24/outline";
import { useNavigation } from "@/lib/navigation-context";
import { agentDisplayName, type Agent } from "@/lib/types";

function AgentIcon({ agent }: { agent: Agent }) {
  const avatar = agent.identity?.avatar;
  if (avatar && (avatar.startsWith("data:") || avatar.startsWith("http") || avatar.startsWith("/api/"))) {
    return (
      <img
        src={avatar}
        alt={agent.name}
        className="h-7 w-7 shrink-0 rounded-full object-cover"
      />
    );
  }
  const name = agentDisplayName(agent.id, agent.name);
  return (
    <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-xs font-medium bg-white/10 text-text-secondary">
      {name.charAt(0).toUpperCase()}
    </div>
  );
}

export function AgentDropdown() {
  const router = useRouter();
  const { agents, deleteAgent } = useNavigation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  const handleDelete = async (e: React.MouseEvent, agent: Agent) => {
    e.stopPropagation();
    if (!confirm(`Delete agent "${agentDisplayName(agent.id, agent.name)}"?`)) return;
    await deleteAgent(agent.id);
  };

  return (
    <div ref={ref} className="relative flex items-center">
      <button
        onClick={() => setOpen((v) => !v)}
        className={`relative flex items-center justify-center h-10 w-10 transition cursor-pointer ${
          open ? "rounded-t-xl rounded-b-none bg-surface-secondary text-text-primary z-[61] border border-border border-b-0" : "rounded-full bg-surface-tertiary text-text-secondary hover:brightness-125"
        }`}
        title="Agents"
      >
        <UserGroupIcon className="h-5 w-5" />
      </button>

      {open && (
        <div className="absolute right-0 top-full z-[60] w-64 rounded-xl rounded-tr-none border border-border bg-surface-secondary shadow-lg">
          <div className="absolute -top-px right-0 w-[calc(theme(spacing.10)-3px)] h-[2px] bg-surface-secondary z-[60]" />
          <div className="pb-1">
            <div className="flex items-center justify-between px-4 py-2 border-b border-border shrink-0">
              <span className="text-sm font-medium text-text-secondary">Agents</span>
            </div>
            {agents.map((agent) => (
              <div
                key={agent.id}
                className="group flex items-center gap-2 px-4 py-2 hover:bg-surface-tertiary transition cursor-pointer"
                onClick={() => {
                  router.push(`/chat?agent=${agent.id}`);
                  setOpen(false);
                }}
              >
                <AgentIcon agent={agent} />
                <span className="flex-1 truncate text-sm text-text-secondary group-hover:text-text-primary">
                  {agentDisplayName(agent.id, agent.name)}
                </span>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    router.push(`/agents?id=${agent.id}`);
                    setOpen(false);
                  }}
                  className="p-1 rounded text-text-tertiary hover:text-text-primary transition opacity-0 group-hover:opacity-100"
                  title="Configure"
                >
                  <Cog6ToothIcon className="h-4 w-4" />
                </button>
                {!agent.is_shared && (
                  <button
                    onClick={(e) => handleDelete(e, agent)}
                    className="p-1 rounded text-text-tertiary hover:text-error-text transition opacity-0 group-hover:opacity-100"
                    title="Delete"
                  >
                    <TrashIcon className="h-4 w-4" />
                  </button>
                )}
              </div>
            ))}
            {agents.length === 0 && (
              <p className="px-3 py-4 text-center text-xs text-text-tertiary">
                No agents configured
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
