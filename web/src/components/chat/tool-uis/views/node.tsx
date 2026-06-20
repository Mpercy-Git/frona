"use client";

import { makeCodeExecView } from "./code-exec";

export const NodeView = makeCodeExecView({
  title: "Node",
  language: "javascript",
  argKey: "code",
  lineNumbers: true,
  failureLabel: "Node failed",
});
