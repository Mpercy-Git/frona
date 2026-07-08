"use client";

import { NavigationPanel } from "@/components/layout/navigation-panel";
import { ConversationPanel } from "@/components/chat/conversation-panel";

export default function ChatLayout() {
  return (
    <div className="flex h-full">
      <NavigationPanel />
      <ConversationPanel />
    </div>
  );
}
