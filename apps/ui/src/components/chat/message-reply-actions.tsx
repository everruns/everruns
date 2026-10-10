"use client";

// Good / Bad response and "Branch into new chat" on an agent reply
// (knowledge/ui/chat-experience.md, message action row).

import { useState } from "react";
import { useRouter } from "next/navigation";
import { GitBranch, Loader2, ThumbsDown, ThumbsUp, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useForkSession } from "@/hooks/use-sessions";
import { useMessageFeedback, useSetMessageFeedback } from "@/hooks/use-message-feedback";
import type { MessageRating } from "@/lib/api/message-feedback";
import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

function ActionButton({
  label,
  icon: Icon,
  pressed,
  disabled,
  busy,
  onClick,
  testId,
}: {
  label: string;
  icon: LucideIcon;
  pressed?: boolean;
  disabled?: boolean;
  busy?: boolean;
  onClick: () => void;
  testId: string;
}) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="-my-1 shrink-0 p-0"
            aria-label={label}
            aria-pressed={pressed}
            disabled={disabled}
            onClick={onClick}
            data-testid={testId}
          >
            {busy ? (
              <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
            ) : (
              <Icon
                className={cn("h-3 w-3", pressed ? "fill-current text-foreground" : "text-muted-foreground")}
              />
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

export function MessageReplyActions({
  sessionId,
  messageId,
  sessionActive,
}: {
  sessionId: string;
  messageId: string;
  /** A running turn cannot be branched; the server refuses a mid-turn fork. */
  sessionActive: boolean;
}) {
  const { t } = useLocale();
  const router = useRouter();
  // Every reply shares one cached request for the session's ratings.
  const { data: ratings } = useMessageFeedback(sessionId);
  const rating = ratings?.get(messageId);
  const setFeedback = useSetMessageFeedback(sessionId);
  const forkSession = useForkSession();
  const [branchError, setBranchError] = useState<string | null>(null);

  // Pressing the active rating again clears it.
  const toggle = (next: MessageRating) =>
    setFeedback.mutate({ messageId, rating: rating === next ? null : next });

  const branch = () => {
    setBranchError(null);
    forkSession.mutate(
      { sessionId, request: { up_to_message_id: messageId } },
      {
        onSuccess: (session) => router.push(`/chats/${session.id}`),
        onError: (error) =>
          setBranchError(error instanceof Error ? error.message : t("branch_chat_error")),
      },
    );
  };

  return (
    <>
      <ActionButton
        label={t("good_response")}
        icon={ThumbsUp}
        pressed={rating === "good"}
        onClick={() => toggle("good")}
        testId="message-feedback-good"
      />
      <ActionButton
        label={t("bad_response")}
        icon={ThumbsDown}
        pressed={rating === "bad"}
        onClick={() => toggle("bad")}
        testId="message-feedback-bad"
      />
      <ActionButton
        label={t("branch_into_new_chat")}
        icon={GitBranch}
        disabled={sessionActive || forkSession.isPending}
        busy={forkSession.isPending}
        onClick={branch}
        testId="message-branch"
      />
      {branchError && (
        <span role="alert" className="text-xs text-destructive">
          {branchError}
        </span>
      )}
    </>
  );
}
