"use client";

import { makeCodeExecView } from "./code-exec";

export const PythonView = makeCodeExecView({
  title: "Python",
  language: "python",
  argKey: "code",
  lineNumbers: true,
  failureLabel: "Python failed",
});
