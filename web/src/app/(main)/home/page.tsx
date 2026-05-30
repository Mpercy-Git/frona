"use client";

import { useCallback } from "react";
import { useRouter } from "next/navigation";
import { AssistantRuntimeProvider } from "@assistant-ui/react";
import { api } from "@/lib/api-client";
import { useAuth } from "@/lib/auth";
import { useChatRuntime } from "@/lib/use-chat-runtime";
import { useSession } from "@/lib/session-context";
import { useNavigation, useSystemAgent } from "@/lib/navigation-context";
import { FronaComposer } from "@/components/chat/frona-composer";
import { Logo } from "@/components/logo";
import { agentDisplayName, type Attachment, type ChatResponse } from "@/lib/types";

const quickActions = [
  { label: "Build an app", prompt: "Help me build a web application" },
  { label: "Analyze data", prompt: "Help me analyze some data" },
  { label: "Create agent", prompt: "I want to create a new agent" },
  { label: "Research", prompt: "Help me research a topic" },
  { label: "Brainstorm", prompt: "Let's brainstorm ideas" },
];

function HomeComposer() {
  const router = useRouter();
  const { addStandaloneChat } = useNavigation();
  const { setPendingMessage } = useSession();
  const systemAgent = useSystemAgent();
  const { runtime } = useChatRuntime({ agentId: systemAgent.id });
  const agentName = agentDisplayName(systemAgent.handle, systemAgent.name);

  const handleSend = useCallback(
    (content: string, attachments?: Attachment[]) => {
      api.post<ChatResponse>("/api/chats", { agent_id: systemAgent.id }).then((chat) => {
        addStandaloneChat(chat);
        setPendingMessage(content, attachments);
        router.push(`/chat?id=${chat.id}`);
      });
    },
    [addStandaloneChat, setPendingMessage, router, systemAgent.id],
  );

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <FronaComposer placeholder={`Ask ${agentName}`} onSend={handleSend} />
      <div className="flex flex-wrap gap-3 mt-5 justify-center">
        {quickActions.map((action) => (
          <button
            key={action.label}
            onClick={() => handleSend(action.prompt)}
            className="px-4 md:px-5 py-2.5 rounded-full border border-border text-sm md:text-base text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition cursor-pointer"
          >
            {action.label}
          </button>
        ))}
      </div>
    </AssistantRuntimeProvider>
  );
}

export default function HomePage() {
  const { user } = useAuth();
  const firstName = user?.name?.split(" ")[0] || user?.email?.split("@")[0];

  return (
    <div className="flex h-full items-center justify-center overflow-y-auto">
      <div className="w-full max-w-3xl px-4 md:px-8 pb-20">
        <div className="mb-5">
          <div className="flex items-end gap-3 mb-2">
            <Logo size={42} headOnly />
            <span className="text-xl md:text-2xl text-text-secondary">
              Hi{firstName ? ` ${firstName}` : ""}
            </span>
          </div>
          <h1 className="text-2xl md:text-4xl font-semibold text-text-primary">
            Where should we start?
          </h1>
        </div>
        <HomeComposer />
      </div>
    </div>
  );
}
