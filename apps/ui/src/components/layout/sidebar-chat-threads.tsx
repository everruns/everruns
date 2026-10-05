/**
 * Live thread list under the sidebar's Chat entry.
 *
 * Two constraints shape it. The list is capped (`SIDEBAR_THREAD_LIMIT`) because
 * live threads in the nav mean the nav is never the same twice, so it has to be
 * bounded. And the order is frozen while the pointer or keyboard focus is inside
 * the list, so an arriving turn cannot re-sort a row out from under a click;
 * the pending order is adopted as soon as the user leaves.
 *
 * This is also where the user's permanent Platform Chat conversation is ensured: the
 * list is rendered on every app route in both the OSS app and its wrappers, so
 * a user who never passes through onboarding (an invited member, say) still
 * finds the thread waiting. The onboarding surfaces, which render without the
 * sidebar, ensure it themselves.
 */
"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { ArrowRight, Pin, Plus } from "lucide-react";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import { SIDEBAR_THREAD_LIMIT, threadTitle } from "@/lib/chat-threads";
import type { Session } from "@/lib/api/types";
import { cn } from "@/lib/utils";

const rowClass =
  "flex items-center gap-2 py-1 px-2 text-xs leading-5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring";

/** Hold `value` steady while `frozen` is true, adopting the latest on thaw. */
function useFrozen<T>(value: T, frozen: boolean): T {
  const [held, setHeld] = useState(value);
  const latest = useRef(value);
  latest.current = value;

  useEffect(() => {
    if (frozen) return;
    setHeld(latest.current);
  }, [frozen, value]);

  return frozen ? held : value;
}

export function SidebarChatThreads({ pathname }: { pathname: string }) {
  const { threads, isLoading } = useChatThreads();
  usePlatformChatThread({ ensure: true });
  const [interacting, setInteracting] = useState(false);
  const stableThreads = useFrozen<Session[]>(threads, interacting);
  const visible = stableThreads.slice(0, SIDEBAR_THREAD_LIMIT);

  if (isLoading && visible.length === 0) return null;

  return (
    <div
      role="group"
      aria-label="Side chats"
      className="ml-9 mr-2.5 mb-1"
      onMouseEnter={() => setInteracting(true)}
      onMouseLeave={() => setInteracting(false)}
      onFocusCapture={() => setInteracting(true)}
      onBlurCapture={() => setInteracting(false)}
    >
      <div className="border-l border-border">
        <div className="flex items-center justify-between pl-2 pr-1 text-muted-foreground">
          <p className="text-[10px] font-medium uppercase tracking-[0.08em]">Side chats</p>
          <Link
            href="/chats/new"
            prefetch={false}
            aria-label="New side chat"
            title="New side chat"
            className="flex size-7 shrink-0 items-center justify-center hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          >
            <Plus className="size-3.5" aria-hidden="true" />
          </Link>
        </div>
        {visible.map((thread) => {
          const href = `/chats/${thread.id}`;
          const isActive = pathname === href;
          return (
            <Link
              key={thread.id}
              href={href}
              prefetch={false}
              aria-current={isActive ? "page" : undefined}
              className={cn(
                rowClass,
                isActive
                  ? "bg-muted font-medium text-foreground"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground",
              )}
            >
              <span className="truncate">{threadTitle(thread)}</span>
              {thread.is_pinned === true && (
                <Pin className="ml-auto size-3 shrink-0 text-primary" aria-label="Pinned" />
              )}
            </Link>
          );
        })}
      </div>

      {stableThreads.length > SIDEBAR_THREAD_LIMIT && (
        <Link
          href="/chats/history"
          prefetch={false}
          className={cn(
            rowClass,
            "mt-1 font-medium text-foreground underline-offset-4 hover:bg-muted hover:underline focus-visible:underline",
          )}
        >
          View all chats
          <ArrowRight className="size-3.5 shrink-0" aria-hidden="true" />
        </Link>
      )}
    </div>
  );
}
