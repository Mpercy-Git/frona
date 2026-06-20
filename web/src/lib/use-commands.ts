"use client";

import { useEffect, useState } from "react";
import { listCommands, type CommandsResponse } from "@/lib/api-client";

const cache = new Map<string, CommandsResponse>();

export function useCommands(chatId: string | undefined): CommandsResponse | null {
  const [data, setData] = useState<CommandsResponse | null>(
    chatId ? (cache.get(chatId) ?? null) : null,
  );

  useEffect(() => {
    if (!chatId) {
      setData(null);
      return;
    }
    const cached = cache.get(chatId);
    if (cached) {
      setData(cached);
      return;
    }
    let cancelled = false;
    listCommands(chatId)
      .then((resp) => {
        if (cancelled) return;
        cache.set(chatId, resp);
        setData(resp);
      })
      .catch((err) => {
        console.warn("Failed to load chat commands:", err);
      });
    return () => {
      cancelled = true;
    };
  }, [chatId]);

  return data;
}
