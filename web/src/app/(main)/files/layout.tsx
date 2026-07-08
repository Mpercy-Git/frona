"use client";

import { NavigationPanel } from "@/components/layout/navigation-panel";
import { useMobile } from "@/lib/use-mobile";

export default function FilesLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const mobile = useMobile();

  return (
    <div className="flex h-full">
      {mobile && <NavigationPanel />}
      <div className="flex-1 overflow-hidden bg-surface min-w-0">
        {children}
      </div>
    </div>
  );
}
