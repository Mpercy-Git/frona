"use client";

export default function SettingsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="flex h-full">
      <div className="flex-1 overflow-hidden bg-surface min-w-0">
        {children}
      </div>
    </div>
  );
}
