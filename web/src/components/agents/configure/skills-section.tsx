"use client";

import { forwardRef } from "react";
import { SectionHeader } from "@/components/settings/field";
import { PuzzlePieceIcon } from "@heroicons/react/24/outline";
import { SkillBrowser, type SkillBrowserHandle } from "@/components/skills/skill-browser";

interface SkillsSectionProps {
  agentId: string;
  skills: string[] | null;
  onSkillsChange: (skills: string[] | null) => void;
  onAgentRemovalsChange?: (hasRemovals: boolean) => void;
}

export const SkillsSection = forwardRef<SkillBrowserHandle, SkillsSectionProps>(
  function SkillsSection({ agentId, skills, onSkillsChange, onAgentRemovalsChange }, ref) {
    return (
      <div>
        <SectionHeader title="Skills" description="Install and manage skills for this agent" icon={PuzzlePieceIcon} />
        <SkillBrowser
          ref={ref}
          agentId={agentId}
          enabledSkills={skills}
          onEnabledChange={onSkillsChange}
          onAgentRemovalsChange={onAgentRemovalsChange}
        />
      </div>
    );
  },
);
