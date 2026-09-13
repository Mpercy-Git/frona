"use client";

import { Suspense, useEffect } from "react";
import { AppGate } from "@/components/app-gate";
import { NavigationProvider } from "@/lib/navigation-context";
import { NotificationProvider } from "@/lib/notification-context";
import { SessionProvider } from "@/lib/session-context";
import { ChatActivityProvider } from "@/lib/chat-activity-context";
import { TopBar } from "@/components/layout/top-bar";
import { registerServiceWorker } from "@/lib/sw-register";

export default function MainLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  useEffect(() => {
    registerServiceWorker();
  }, []);

  return (
    <NavigationProvider>
      <AppGate>
        <NotificationProvider>
          <Suspense>
            <SessionProvider>
              <ChatActivityProvider>
                <div className="flex flex-col h-[100dvh]">
                  <TopBar />
                  <div className="flex-1 overflow-hidden">
                    {children}
                  </div>
                </div>
              </ChatActivityProvider>
            </SessionProvider>
          </Suspense>
        </NotificationProvider>
      </AppGate>
    </NavigationProvider>
  );
}