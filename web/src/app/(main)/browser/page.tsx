"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { BrowserLiveView } from "@/components/browser/live-view";

/** Static page — `output: "export"` rules out dynamic routes — so the browser
 *  profile arrives as `?profile=`. Without one, the server opens the profile
 *  the agent is currently using. */
function Inner() {
  const profile = useSearchParams().get("profile");
  return <BrowserLiveView profile={profile} />;
}

export default function BrowserPage() {
  return (
    <Suspense>
      <Inner />
    </Suspense>
  );
}
