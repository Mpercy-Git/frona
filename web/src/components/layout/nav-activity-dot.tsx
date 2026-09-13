"use client";

import { useChatActivity } from "@/lib/chat-activity-context";
import { ActivityDot } from "@/components/ui/activity-indicator";

/// Stands in for the whole list when it is out of view: something, somewhere
/// in here is working or waiting on you. Working wins when both are true — it
/// is the state that is still changing.
export function NavActivityDot() {
  const { workingCount, waitingCount } = useChatActivity();
  if (workingCount > 0) {
    return (
      <ActivityDot
        activity="working"
        label={`${workingCount} conversation${workingCount === 1 ? "" : "s"} being worked on`}
      />
    );
  }
  if (waitingCount > 0) {
    return (
      <ActivityDot
        activity="waiting"
        label={`${waitingCount} conversation${waitingCount === 1 ? "" : "s"} waiting for your answer`}
      />
    );
  }
  return null;
}
