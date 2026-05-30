"use client";

import { useEffect, useCallback, Suspense } from "react";
import { useRouter, useSearchParams, redirect } from "next/navigation";
import { AssistantRuntimeProvider } from "@assistant-ui/react";
import { api } from "@/lib/api-client";
import { useSession } from "@/lib/session-context";
import { useNavigation, useSystemAgent } from "@/lib/navigation-context";
import { useChatRuntime } from "@/lib/use-chat-runtime";
import { FronaComposer } from "@/components/chat/frona-composer";
import type { Attachment, ChatResponse } from "@/lib/types";

function SpaceComposer({ spaceId }: { spaceId: string }) {
  const router = useRouter();
  const { refresh } = useNavigation();
  const { setPendingMessage } = useSession();
  const systemAgent = useSystemAgent();
  const { runtime } = useChatRuntime({ agentId: systemAgent.id });

  const handleSend = useCallback((content: string, attachments?: Attachment[]) => {
    api.post<ChatResponse>("/api/chats", { space_id: spaceId, agent_id: systemAgent.id }).then((chat) => {
      refresh();
      setPendingMessage(content, attachments);
      router.push(`/chat?id=${chat.id}`);
    });
  }, [spaceId, refresh, setPendingMessage, router, systemAgent.id]);

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <FronaComposer placeholder="Send a message to start a new chat..." onSend={handleSend} />
    </AssistantRuntimeProvider>
  );
}

function SpaceView({ spaceId }: { spaceId: string }) {
  const { spaces } = useNavigation();
  const { activeChatId } = useSession();
  const router = useRouter();

  const space = spaces.find((s) => s.id === spaceId);

  useEffect(() => {
    if (!space) router.push("/home");
  }, [space, router]);

  if (!space) return null;

  return (
    <div className="flex flex-1 flex-col">
      <div className="mx-auto flex w-full max-w-3xl flex-1 flex-col">
        <div className="border-b border-border px-6 py-4">
          <h2 className="text-2xl font-bold text-text-primary">{space.name}</h2>
        </div>

        <div className="px-6 py-4">
          <SpaceComposer spaceId={spaceId} />
        </div>

        <div className="flex-1 overflow-y-auto px-6">
          {space.chats.length > 0 && (
            <div className="space-y-1">
              <p className="text-[11px] font-semibold uppercase tracking-wider text-text-tertiary pb-1">
                Chats
              </p>
              {space.chats.map((chat) => (
                <button
                  key={chat.id}
                  onClick={() => router.push(`/chat?id=${chat.id}`)}
                  className={`w-full rounded-lg px-4 py-2.5 text-left text-sm transition truncate ${
                    activeChatId === chat.id
                      ? "bg-surface-tertiary text-text-primary"
                      : "text-text-secondary hover:bg-surface-secondary"
                  }`}
                >
                  {chat.title ?? "New chat"}
                </button>
              ))}
            </div>
          )}
          {space.chats.length === 0 && (
            <p className="py-8 text-center text-sm text-text-tertiary">
              No chats in this space yet. Type above to start one.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

export default function SpacePage() {
  return (
    <Suspense>
      <SpacePageContent />
    </Suspense>
  );
}

function SpacePageContent() {
  const searchParams = useSearchParams();
  const spaceId = searchParams.get("id");
  if (!spaceId) redirect("/home");
  return <SpaceView spaceId={spaceId} />;
}
