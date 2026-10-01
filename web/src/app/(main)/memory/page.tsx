import { Suspense } from "react";
import { MemoryPage } from "@/components/memory/memory-page";

export default function Page() {
  return (
    <Suspense fallback={<div className="flex h-full items-center justify-center text-sm text-text-secondary">Loading memory…</div>}>
      <MemoryPage />
    </Suspense>
  );
}
