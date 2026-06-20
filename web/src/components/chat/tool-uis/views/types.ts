import type { FC } from "react";
import type { ToolCallMessagePartProps } from "@assistant-ui/react";

export type ToolViewProps = ToolCallMessagePartProps & {
  isExpanded: boolean;
  onToggle: () => void;
};

export type ToolView = FC<ToolViewProps>;
