"use client";

import { useEffect } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";

// Chat is the landing route (knowledge/ui/information-architecture.md). Landing
// opens Main chat beside Overview instead of full width, so the first page Chat
// can read is the org's attention list (knowledge/ui/org-home.md). `/` marks the
// arrival with `?landing`; the shell redirects once the organization is ready.
export const LANDING_PARAM = "landing";
/** Opens the page-side Chat on arrival; also usable as a deep link. */
export const OPEN_CHAT_PARAM = "chat";

export function landingTarget(): string {
  return `/home?${OPEN_CHAT_PARAM}=open`;
}

export function useLandingRedirect(ready: boolean) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const router = useRouter();
  const landing = pathname === "/chats" && searchParams.has(LANDING_PARAM);
  useEffect(() => {
    if (landing && ready) router.replace(landingTarget());
  }, [landing, ready, router]);
}

/** Drop `?chat=open` once the split has opened, keeping any other query. */
export function withoutOpenChat(pathname: string, searchParams: URLSearchParams): string {
  const params = new URLSearchParams(searchParams.toString());
  params.delete(OPEN_CHAT_PARAM);
  const query = params.toString();
  return query ? `${pathname}?${query}` : pathname;
}
