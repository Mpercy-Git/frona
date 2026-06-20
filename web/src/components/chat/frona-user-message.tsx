"use client";

import { useCallback, useEffect, useState } from "react";
import { MessagePrimitive, AttachmentPrimitive, useMessage } from "@assistant-ui/react";
import type { CompleteAttachment } from "@assistant-ui/react";
import { presignFile } from "@/lib/api-client";
import { getBackendAttachment } from "@/lib/use-chat-runtime";
import { formatFullTimestamp, formatTime, useTimezone } from "@/lib/format-time";
import { useNavigation } from "@/lib/navigation-context";
import type { MessageCommand } from "@/lib/types";
import { MarkdownText } from "./markdown-text";
import { TimeMarkers } from "./time-markers";

function InvocationChip({ command, prefix }: { command: MessageCommand; prefix: string }) {
  const { agents } = useNavigation();
  const tail = (command.type === "skill" ? command.prompt : command.args).trim();

  const matchedAgent =
    command.type === "command"
      ? agents.find((a) => a.handle === command.name)
      : undefined;
  const displayName = matchedAgent?.name ?? command.name;

  const directiveType =
    command.type === "skill"
      ? "skill"
      : matchedAgent || prefix === "@"
        ? "agent"
        : "command";
  return (
    <div className="text-base text-text-primary">
      <span className="aui-directive-chip" data-directive-type={directiveType}>
        {prefix}{displayName}
      </span>
      {tail && <span>{tail}</span>}
    </div>
  );
}

function CommandTypeBadge({ command }: { command: MessageCommand }) {
  const { agents } = useNavigation();
  const isAgent =
    command.type === "command" && agents.some((a) => a.handle === command.name);
  if (isAgent) return null;
  const label = command.type === "skill" ? "Skill" : "Command";
  return (
    <span className="inline-flex items-center rounded-full bg-surface-tertiary px-1.5 py-px text-[9px] font-medium uppercase tracking-wide text-text-tertiary">
      {label}
    </span>
  );
}

function usePresignedUrl(attachmentId: string) {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    const backend = getBackendAttachment(attachmentId);
    if (!backend) return;
    let cancelled = false;
    presignFile(backend.owner, backend.path).then((u) => {
      if (!cancelled) setUrl(u);
    }).catch(() => {});
    return () => { cancelled = true; };
  }, [attachmentId]);

  return url;
}

function UserAttachment({ attachment }: { attachment: CompleteAttachment }) {
  const isImage = attachment.type === "image" || attachment.contentType?.startsWith("image/");
  const presignedUrl = usePresignedUrl(attachment.id);

  const handleClick = useCallback(() => {
    if (presignedUrl) window.open(presignedUrl, "_blank");
  }, [presignedUrl]);

  if (isImage && presignedUrl) {
    return (
      <AttachmentPrimitive.Root
        className="overflow-hidden rounded-lg border border-border cursor-pointer hover:opacity-90 transition-opacity"
        onClick={handleClick}
      >
        <img
          src={presignedUrl}
          alt={attachment.name}
          className="max-h-72 max-w-full object-contain"
        />
      </AttachmentPrimitive.Root>
    );
  }

  if (isImage) {
    return (
      <AttachmentPrimitive.Root className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-tertiary px-3 py-2 text-xs text-text-secondary">
        <AttachmentPrimitive.Name />
      </AttachmentPrimitive.Root>
    );
  }

  return (
    <AttachmentPrimitive.Root
      className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface-tertiary px-3 py-2 text-xs text-text-secondary cursor-pointer hover:bg-surface-secondary transition-colors"
      onClick={handleClick}
    >
      <AttachmentPrimitive.Name />
    </AttachmentPrimitive.Root>
  );
}

export function FronaUserMessage() {
  const message = useMessage();
  const tz = useTimezone();
  const custom = (message.metadata as Record<string, any>)?.custom ?? {};
  const command: MessageCommand | undefined = custom.command;
  const rawText =
    (message.content[0] as { type?: string; text?: string } | undefined)?.type === "text"
      ? ((message.content[0] as { text?: string }).text ?? "")
      : "";
  const prefix = rawText.charAt(0) === "@" ? "@" : "/";
  const isoTime = message.createdAt?.toISOString();
  return (
    <MessagePrimitive.Root>
      <TimeMarkers daySeparator={custom.daySeparator} gap={custom.gap} />
      <div className="group w-full" data-message-id={message.id}>
        <div className="flex items-center gap-2.5 h-8">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-xs font-medium bg-accent text-surface">
            Y
          </div>
          <p className="text-xs font-medium text-text-tertiary">You</p>
          {command && <CommandTypeBadge command={command} />}
          {isoTime && (
            <time
              dateTime={isoTime}
              title={formatFullTimestamp(isoTime, tz)}
              className="text-xs text-text-tertiary opacity-0 group-hover:opacity-100 transition"
            >
              {formatTime(isoTime, tz)}
            </time>
          )}
        </div>
        <div className="pl-[42px] text-base text-text-primary">
          {command ? (
            <InvocationChip command={command} prefix={prefix} />
          ) : (
            <MessagePrimitive.Parts
              components={{
                Text: MarkdownText,
              }}
            />
          )}
          <div className="flex flex-wrap gap-2 mt-2 empty:hidden">
            <MessagePrimitive.Attachments>
              {({ attachment }) => <UserAttachment attachment={attachment} />}
            </MessagePrimitive.Attachments>
          </div>
        </div>
      </div>
    </MessagePrimitive.Root>
  );
}
