"use client";

import { NavigationPanel } from "@/components/layout/navigation-panel";

export default function HomeLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="flex h-full">
      <NavigationPanel />
      <div className="flex-1 overflow-hidden bg-surface">
        {children}
      </div>
    </div>
  );
}
