"use client";

import { ChatBubbleLeftRightIcon, ClipboardDocumentListIcon } from "@heroicons/react/24/outline";
import { useNavigation } from "@/lib/navigation-context";

const tabs = [
  { id: "chat" as const, label: "Chat", Icon: ChatBubbleLeftRightIcon },
  { id: "tasks" as const, label: "Tasks", Icon: ClipboardDocumentListIcon },
];

export function TabBar() {
  const { activeTab, setActiveTab } = useNavigation();

  return (
    <div className="flex">
      {tabs.map(({ id, label, Icon }) => (
        <button
          key={id}
          onClick={() => setActiveTab(id)}
          className={`flex-1 flex items-center justify-center gap-1.5 py-3 md:py-2.5 text-sm md:text-xs font-medium transition ${
            activeTab === id
              ? "text-text-primary border-b-2 border-text-primary"
              : "text-text-tertiary hover:text-text-secondary"
          }`}
        >
          <Icon className="h-5 w-5 md:h-4 md:w-4" />
          {label}
        </button>
      ))}
    </div>
  );
}
