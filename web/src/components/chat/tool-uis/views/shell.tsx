"use client";

import { makeCodeExecView } from "./code-exec";
import { summarizeCommand } from "./shell-summarize";

export const ShellView = makeCodeExecView({
  title: "Shell",
  language: "bash",
  wrap: true,
  argKey: "command",
  failureLabel: "Command failed",
  summarize: summarizeCommand,
});
