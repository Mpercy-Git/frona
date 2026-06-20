"use client";

import { api } from "@/lib/api-client";
import type { ChatResponse } from "@/lib/types";

export interface BuiltinContext {
  chatId?: string;
  agentId?: string;
  router: { push: (path: string) => void };
}

export interface ClientBuiltin {
  name: string;
  description: string;
  argument_hint?: string;
  run: (args: string, ctx: BuiltinContext) => void | Promise<void>;
}

const newBuiltin: ClientBuiltin = {
  name: "new",
  description: "Start a fresh chat with the current agent.",
  run: async (_args, ctx) => {
    if (!ctx.agentId) return;
    const chat = await api.post<ChatResponse>("/api/chats", { agent_id: ctx.agentId });
    ctx.router.push(`/chat?id=${chat.id}`);
  },
};

export const CLIENT_BUILTINS: ClientBuiltin[] = [newBuiltin];

export function findClientBuiltin(name: string): ClientBuiltin | undefined {
  return CLIENT_BUILTINS.find((b) => b.name === name.toLowerCase());
}

/** Returns true if dispatched; caller falls through to normal POST otherwise. */
export async function tryDispatchClientBuiltin(
  content: string,
  ctx: BuiltinContext,
): Promise<boolean> {
  if (!content.startsWith("/")) return false;
  const trimmed = content.slice(1);
  const sep = trimmed.search(/\s/);
  const name = sep === -1 ? trimmed : trimmed.slice(0, sep);
  const args = sep === -1 ? "" : trimmed.slice(sep + 1);
  const handler = findClientBuiltin(name);
  if (!handler) return false;
  await handler.run(args, ctx);
  return true;
}
