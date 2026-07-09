"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  ChartBarIcon,
  CurrencyDollarIcon,
  ClockIcon,
  CpuChipIcon,
} from "@heroicons/react/24/outline";

import { NavigationPanel } from "@/components/layout/navigation-panel";
import { useMobile } from "@/lib/use-mobile";

import { RANGES, UsageProvider, useUsage } from "./usage-context";

const NAV = [
  { href: "/usage", label: "Overview", icon: ChartBarIcon, exact: true },
  { href: "/usage/cost", label: "Cost", icon: CurrencyDollarIcon },
  { href: "/usage/latency", label: "Latency", icon: ClockIcon },
  { href: "/usage/tokens", label: "Tokens", icon: CpuChipIcon },
];

export default function UsageLayout({ children }: { children: React.ReactNode }) {
  const mobile = useMobile();
  return (
    <UsageProvider>
      <div className="flex h-full">
        {mobile && <NavigationPanel />}
        <div className="flex flex-1 overflow-hidden bg-surface min-w-0">
          {!mobile && <UsageSidebar />}
          <div className="flex-1 overflow-auto">
            <RangeBar />
            {children}
          </div>
        </div>
      </div>
    </UsageProvider>
  );
}

function UsageSidebar() {
  const pathname = usePathname();
  return (
    <aside className="w-56 shrink-0 border-r border-border bg-surface-secondary px-3 py-6">
      <div className="px-3 pb-2 text-xs font-semibold uppercase tracking-wide text-text-tertiary">
        Usage
      </div>
      <nav className="flex flex-col gap-0.5">
        {NAV.map((item) => {
          const Icon = item.icon;
          const active = item.exact
            ? pathname === item.href
            : pathname.startsWith(item.href);
          return (
            <Link
              key={item.href}
              href={item.href}
              className={`flex items-center gap-2 rounded-md px-3 py-1.5 text-sm transition ${
                active
                  ? "bg-surface text-text-primary"
                  : "text-text-secondary hover:bg-surface hover:text-text-primary"
              }`}
            >
              <Icon className="h-4 w-4 shrink-0" />
              {item.label}
            </Link>
          );
        })}
      </nav>
    </aside>
  );
}

function RangeBar() {
  const { rangeIdx, setRangeIdx } = useUsage();
  return (
    <div className="sticky top-0 z-10 flex items-center justify-end border-b border-border bg-surface/80 px-4 py-2 backdrop-blur md:px-8">
      <div className="inline-flex rounded-lg border border-border bg-surface-secondary p-0.5">
        {RANGES.map((r, i) => (
          <button
            key={r.label}
            type="button"
            onClick={() => setRangeIdx(i)}
            className={`rounded-md px-3 py-1 text-sm transition ${
              i === rangeIdx
                ? "bg-surface text-text-primary shadow-sm"
                : "text-text-tertiary hover:text-text-secondary"
            }`}
          >
            {r.label}
          </button>
        ))}
      </div>
    </div>
  );
}
