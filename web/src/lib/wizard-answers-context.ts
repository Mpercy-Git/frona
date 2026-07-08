"use client";

import { createContext, useContext } from "react";
import type { WizardAnswer } from "@/components/chat/external-tool-drawer";

export const WizardAnswersContext = createContext<Map<string, WizardAnswer>>(new Map());

export function useWizardAnswers(): Map<string, WizardAnswer> {
  return useContext(WizardAnswersContext);
}
