"use client";

import {
  Children,
  createContext,
  isValidElement,
  useContext,
  type ReactElement,
  type ReactNode,
} from "react";
import { ChevronDownIcon } from "@heroicons/react/24/outline";
import { AnimatePresence, motion } from "motion/react";
import type { ToolCallMessagePartStatus } from "@assistant-ui/react";
import { cn } from "@/lib/utils";

export type Status = ToolCallMessagePartStatus | undefined;

const SLOT = Symbol.for("toolrow.slot");
type SlotName = "header" | "body" | "error";
type SlotMarker = { [SLOT]?: SlotName };

const ANIMATION_DURATION = 200;

type ToolRowCtxValue = {
  status: Status;
  expandable: boolean;
  errorSlot: ReactNode | null;
};

const ToolRowCtx = createContext<ToolRowCtxValue | null>(null);

function useToolRow(): ToolRowCtxValue {
  const v = useContext(ToolRowCtx);
  if (!v) throw new Error("ToolRow.* must be rendered inside <ToolRow>");
  return v;
}

function isCancelled(s: Status): boolean {
  return s?.type === "incomplete" && s.reason === "cancelled";
}

function isErrored(s: Status): boolean {
  return s?.type === "incomplete" && s.reason !== "cancelled";
}

function findSlot(children: ReactNode, name: SlotName): ReactElement | null {
  let found: ReactElement | null = null;
  Children.forEach(children, (c) => {
    if (!isValidElement(c)) return;
    const marker = (c.type as SlotMarker)[SLOT];
    if (marker === name) found = c;
  });
  return found;
}

export interface ToolRowProps {
  status: Status;
  expandable?: boolean;
  children: ReactNode;
}

export function ToolRow({ status, expandable = true, children }: ToolRowProps) {
  const header = findSlot(children, "header");
  const body = findSlot(children, "body");
  const errorSlot = findSlot(children, "error");

  if (process.env.NODE_ENV !== "production" && !header) {
    console.warn("ToolRow: missing <ToolRow.Header>");
  }

  const errorChildren = errorSlot
    ? (errorSlot.props as { children?: ReactNode }).children ?? null
    : null;

  return (
    <ToolRowCtx.Provider value={{ status, expandable, errorSlot: errorChildren }}>
      {header}
      {body}
    </ToolRowCtx.Provider>
  );
}

interface HeaderProps {
  onToggle: () => void;
  isExpanded: boolean;
  children: ReactNode;
}

function Header({ onToggle, isExpanded, children }: HeaderProps) {
  const { status, expandable } = useToolRow();
  const cancelled = isCancelled(status);
  return (
    <button
      type="button"
      onClick={expandable ? onToggle : undefined}
      disabled={!expandable}
      className={cn(
        "flex w-full items-center gap-1.5 text-sm transition-colors",
        cancelled
          ? "text-text-tertiary line-through"
          : "text-text-secondary",
      )}
    >
      <span className="grow min-w-0 text-left leading-snug truncate">{children}</span>
      {expandable && (
        <motion.span
          animate={{ rotate: isExpanded ? 0 : -90 }}
          transition={{ duration: ANIMATION_DURATION / 1000, ease: "easeOut" }}
          className="shrink-0 text-text-tertiary"
        >
          <ChevronDownIcon className="h-3.5 w-3.5" />
        </motion.span>
      )}
    </button>
  );
}
(Header as SlotMarker)[SLOT] = "header";

function Title({ children }: { children: ReactNode }) {
  return <b>{children}</b>;
}

function Subtitle({ children }: { children?: ReactNode }) {
  if (children == null || children === "" || children === false) return null;
  return (
    <span className="font-normal text-text-tertiary"> — {children}</span>
  );
}

interface BodyProps {
  isExpanded: boolean;
  /** Override the default body padding/border styles (used when an inner view supplies its own chrome). */
  unstyled?: boolean;
  children: ReactNode;
}

function Body({ isExpanded, unstyled = false, children }: BodyProps) {
  const { status, errorSlot } = useToolRow();
  const cancelled = isCancelled(status);
  const errored = isErrored(status);

  return (
    <AnimatePresence initial={false}>
      {isExpanded && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ duration: ANIMATION_DURATION / 1000, ease: "easeOut" }}
          className="overflow-hidden"
        >
          <div
            className={cn(
              "mt-2 flex flex-col gap-2 rounded-md border border-border bg-surface-secondary text-sm",
              unstyled ? "p-0 overflow-hidden" : "p-3",
              cancelled && "opacity-60",
            )}
          >
            {cancelled && <DefaultErrorBlock status={status} cancelled />}
            {errored && (errorSlot ?? <DefaultErrorBlock status={status} cancelled={false} />)}
            {!errored && children}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
(Body as SlotMarker)[SLOT] = "body";

function ErrorSlot({ children: _children }: { children: ReactNode }) {
  // Marker-only: ToolRow extracts children and routes them through Body.
  return null;
}
(ErrorSlot as SlotMarker)[SLOT] = "error";

function DefaultErrorBlock({
  status,
  cancelled,
}: {
  status: Status;
  cancelled: boolean;
}) {
  const error =
    status?.type === "incomplete"
      ? (status as { error?: unknown }).error
      : undefined;
  if (error == null) return null;
  const text = typeof error === "string" ? error : JSON.stringify(error);
  return (
    <div className="text-xs p-3">
      <p className="font-semibold text-danger">
        {cancelled ? "Cancelled" : "Failed"}
      </p>
      <pre className="whitespace-pre-wrap text-text-tertiary mt-1">{text}</pre>
    </div>
  );
}

ToolRow.Header = Header;
ToolRow.Title = Title;
ToolRow.Subtitle = Subtitle;
ToolRow.Body = Body;
ToolRow.Error = ErrorSlot;
