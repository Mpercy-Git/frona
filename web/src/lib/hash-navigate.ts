"use client";

import { useRouter } from "next/navigation";

/**
 * Returns a navigation function that handles hash-only changes correctly.
 *
 * Next.js App Router's `router.push("/x#a")` doesn't reliably replace the
 * fragment when you're already on `/x`. This helper detects the same-path
 * case and pushes the new fragment directly + dispatches a `hashchange`
 * event so pages listening for it can react. Cross-route navigation still
 * goes through `router.push`.
 */
export function useHashNavigate(): (href: string) => void {
  const router = useRouter();
  return (href: string) => {
    const [path, hash] = href.split("#");
    if (typeof window !== "undefined" && window.location.pathname === path && hash) {
      window.history.pushState(null, "", `#${hash}`);
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    } else {
      router.push(href);
    }
  };
}
