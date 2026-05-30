"use client";

import { Suspense } from "react";
import { AppGate } from "@/components/app-gate";
import { NavigationProvider } from "@/lib/navigation-context";
import { NotificationProvider } from "@/lib/notification-context";
import { SessionProvider } from "@/lib/session-context";
import { TopBar } from "@/components/layout/top-bar";

export default function MainLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <NavigationProvider>
      <AppGate>
        <NotificationProvider>
          <Suspense>
            <SessionProvider>
              <div className="flex flex-col h-screen">
                <TopBar />
                <div className="flex-1 overflow-hidden">
                  {children}
                </div>
              </div>
            </SessionProvider>
          </Suspense>
        </NotificationProvider>
      </AppGate>
    </NavigationProvider>
  );
}
