"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import type { ComponentProps, ReactNode } from "react";

/**
 * EVE-785: The empty dashboard renders three links to `/agents/new` (Active
 * Agents header, empty state, Quick Actions). With Next.js default (viewport)
 * prefetch, a cold production load eagerly fetched the agent-creation RSC
 * payload before any user intent. Mirror the EVE-780 sidebar policy: disable
 * automatic prefetch and prefetch once on hover/focus intent instead. Next.js
 * dedupes `router.prefetch` per route, so repeated links never duplicate work.
 */
const NEW_AGENT_HREF = "/agents/new";

type NewAgentLinkProps = Omit<ComponentProps<typeof Link>, "href" | "prefetch"> & {
  children: ReactNode;
};

export function NewAgentLink({ children, onMouseEnter, onFocus, ...rest }: NewAgentLinkProps) {
  const router = useRouter();

  return (
    <Link
      href={NEW_AGENT_HREF}
      prefetch={false}
      onMouseEnter={(event) => {
        router.prefetch(NEW_AGENT_HREF);
        onMouseEnter?.(event);
      }}
      onFocus={(event) => {
        router.prefetch(NEW_AGENT_HREF);
        onFocus?.(event);
      }}
      {...rest}
    >
      {children}
    </Link>
  );
}
