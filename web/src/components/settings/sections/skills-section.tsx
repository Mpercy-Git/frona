"use client";

import { PuzzlePieceIcon } from "@heroicons/react/24/outline";
import { SectionHeader } from "../field";
import { SkillBrowser } from "@/components/skills/skill-browser";
import type { InstallScope } from "@/lib/api-client";

interface SkillsSectionProps {
  scope?: InstallScope;
}

export function SkillsSection({ scope = "user" }: SkillsSectionProps) {
  const description = scope === "shared"
    ? "Server-wide skills installed here are available to every user."
    : "Search and install skills from the community. These are scoped to your account.";

  return (
    <div className="space-y-6">
      <SectionHeader
        title="Skills"
        description={description}
        icon={PuzzlePieceIcon}
      />
      <SkillBrowser scope={scope} />
    </div>
  );
}
