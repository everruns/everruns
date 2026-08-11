"use client";

import { Pin, PinOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { usePinSession, useUnpinSession } from "@/hooks/use-sessions";
import { cn } from "@/lib/utils";
import type { Session } from "@/lib/api/types";

export function ChatPinButton({
  session,
  showLabel = false,
  className,
}: {
  session: Pick<Session, "id" | "is_pinned">;
  showLabel?: boolean;
  className?: string;
}) {
  const pinSession = usePinSession();
  const unpinSession = useUnpinSession();
  const isPinned = session.is_pinned === true;
  const isPending = pinSession.isPending || unpinSession.isPending;
  const label = isPinned ? "Unpin chat" : "Pin chat";

  return (
    <Button
      type="button"
      variant="outline"
      size={showLabel ? "sm" : "icon-sm"}
      className={cn(showLabel && "max-sm:w-7 max-sm:px-0", isPinned && "text-primary", className)}
      aria-label={label}
      aria-pressed={isPinned}
      title={label}
      disabled={isPending}
      onClick={() => {
        const mutation = isPinned ? unpinSession : pinSession;
        mutation.mutate({ sessionId: session.id });
      }}
    >
      {isPinned ? <PinOff className="size-4" /> : <Pin className="size-4" />}
      {showLabel && <span className="max-sm:hidden">{label}</span>}
    </Button>
  );
}
