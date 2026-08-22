/**
 * Archive / unarchive a chat thread.
 *
 * Archiving is the non-destructive "I'm done with this" — the thread and its
 * transcript stay intact and reachable by URL, it just stops competing for
 * attention in the thread list. Unlike pinning, it is a property of the session
 * rather than of the viewer, so the label is deliberately about the thread
 * ("Archive chat"), not about a personal view.
 */
"use client";

import { Archive, ArchiveRestore } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useArchiveSession, useUnarchiveSession } from "@/hooks/use-sessions";
import { cn } from "@/lib/utils";
import type { Session } from "@/lib/api/types";

export function ChatArchiveButton({
  session,
  showLabel = false,
  className,
}: {
  session: Pick<Session, "id" | "archived_at">;
  showLabel?: boolean;
  className?: string;
}) {
  const archiveSession = useArchiveSession();
  const unarchiveSession = useUnarchiveSession();
  const isArchived = !!session.archived_at;
  const isPending = archiveSession.isPending || unarchiveSession.isPending;
  const label = isArchived ? "Unarchive chat" : "Archive chat";

  return (
    <Button
      type="button"
      variant="outline"
      size={showLabel ? "sm" : "icon-sm"}
      className={cn(showLabel && "max-sm:w-7 max-sm:px-0", className)}
      aria-label={label}
      aria-pressed={isArchived}
      title={label}
      disabled={isPending}
      onClick={() => {
        const mutation = isArchived ? unarchiveSession : archiveSession;
        mutation.mutate({ sessionId: session.id });
      }}
    >
      {isArchived ? <ArchiveRestore className="size-4" /> : <Archive className="size-4" />}
      {showLabel && <span className="max-sm:hidden">{label}</span>}
    </Button>
  );
}
