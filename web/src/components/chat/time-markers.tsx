"use client";

interface MarkersProps {
  daySeparator?: string;
  gap?: string;
}

export function TimeMarkers({ daySeparator, gap }: MarkersProps) {
  if (!daySeparator && !gap) return null;
  return (
    <div className="flex flex-col items-center gap-1 py-2">
      {daySeparator && (
        <div className="rounded-full bg-surface-secondary px-3 py-0.5 text-xs font-medium text-text-secondary">
          {daySeparator}
        </div>
      )}
      {gap && (
        <div className="text-xs text-text-tertiary">{gap}</div>
      )}
    </div>
  );
}
