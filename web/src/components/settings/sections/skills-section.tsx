"use client";

import { PuzzlePieceIcon } from "@heroicons/react/24/outline";
import { SectionHeader } from "../field";
import { SkillBrowser } from "@/components/skills/skill-browser";

export function SkillsSection() {
  return (
    <div className="space-y-6">
      <SectionHeader
        title="Skills"
        description="Search and install skills from the community"
        icon={PuzzlePieceIcon}
      />
      <SkillBrowser />
    </div>
  );
}
