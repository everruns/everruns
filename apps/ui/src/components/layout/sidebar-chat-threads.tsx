/**
 * Live thread list under the sidebar's Chats entry.
 *
 * Two constraints shape it. The list is capped (`SIDEBAR_THREAD_LIMIT`) because
 * live threads in the nav mean the nav is never the same twice, so it has to be
 * bounded. And the order is frozen while the pointer or keyboard focus is inside
 * the list, so an arriving turn cannot re-sort a row out from under a click;
 * the pending order is adopted as soon as the user leaves.
 */
"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { Pin, Plus } from "lucide-react";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { SIDEBAR_THREAD_LIMIT, threadTitle } from "@/lib/chat-threads";
import type { Session } from "@/lib/api/types";
import { cn } from "@/lib/utils";

const rowClass =
  "flex items-center gap-2 border-l-2 py-1 pl-9 pr-3 text-[13px] leading-5 transition-colors";

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
  const [interacting, setInteracting] = useState(false);
  const stableThreads = useFrozen<Session[]>(threads, interacting);
  const visible = stableThreads.slice(0, SIDEBAR_THREAD_LIMIT);

  if (isLoading && visible.length === 0) return null;

  return (
    <div
      onMouseEnter={() => setInteracting(true)}
      onMouseLeave={() => setInteracting(false)}
      onFocusCapture={() => setInteracting(true)}
      onBlurCapture={() => setInteracting(false)}
    >
      {visible.map((thread) => {
        const href = `/chats/${thread.id}`;
        const isActive = pathname === href;
        return (
          <Link
            key={thread.id}
            href={href}
            prefetch={false}
            className={cn(
              rowClass,
              isActive
                ? "border-l-primary bg-card font-medium text-foreground"
                : "border-l-transparent text-muted-foreground hover:border-l-border hover:bg-card/80 hover:text-foreground",
            )}
          >
            <span className="truncate">{threadTitle(thread)}</span>
            {thread.is_pinned === true && (
              <Pin className="ml-auto size-3 shrink-0 text-primary" aria-label="Pinned" />
            )}
          </Link>
        );
      })}

      <Link
        href="/chats/new"
        prefetch={false}
        className={cn(rowClass, "border-l-transparent text-muted-foreground hover:text-foreground")}
      >
        <Plus className="size-3.5 shrink-0" />
        New chat
      </Link>

      {/* Always offered, even with nothing above it: the all-chats page is the
          only place archived threads can be brought back into view. */}
      <Link
        href="/chats"
        prefetch={false}
        className={cn(rowClass, "border-l-transparent text-muted-foreground hover:text-foreground")}
      >
        All chats
      </Link>
    </div>
  );
}
