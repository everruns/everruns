"use client";

import { useEffect } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import type { AppLayout } from "@/hooks/use-layout-preference";

// Chat is the landing route (knowledge/ui/information-architecture.md). `/`
// marks the arrival with `/chats?landing`; once the organization is ready the
// shell lands chat-first people on Overview (Chat is beside every page there,
// so its first page is the org's attention list, knowledge/ui/org-home.md) and
// classic people on full-screen Chat.
export const LANDING_PARAM = "landing";

export function landingTarget(layout: AppLayout): string {
  return layout === "chat_first" ? "/home" : "/chats";
}

export function useLandingRedirect(ready: boolean, layout: AppLayout) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const router = useRouter();
  const landing = pathname === "/chats" && searchParams.has(LANDING_PARAM);
  useEffect(() => {
    if (landing && ready) router.replace(landingTarget(layout));
  }, [landing, ready, router, layout]);
}
