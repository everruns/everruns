"use client";

// Transcript is the default human-readable view of a session recording. It
// intentionally reuses the chat transcript so completed replay and live output
// stay identical to Chats, but it never renders a composer. An empty recording
// offers only the same fork escape hatch as the session header.
import { Bot } from "lucide-react";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { SessionForkButton } from "@/components/session/session-fork-button";

import { SessionTranscript } from "@/components/session/session-transcript";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import { useLocale } from "@/providers/locale-provider";

export default function TranscriptPage() {
  const { sessionId, session } = useSessionContext();
  const { t } = useLocale();

  return (
    <SessionTranscript
      emptyState={
        <div className={chatSurfaceStyles.emptyStateCard}>
          <div className="mx-auto mb-4 flex h-10 w-10 items-center justify-center border border-border/70 bg-background text-muted-foreground">
            <Bot className="h-5 w-5 opacity-65" />
          </div>
          <p className="text-lg font-medium text-foreground">{t("no_messages_yet")}</p>
          <p className="mt-1 text-sm">{t("session_transcript_empty_description")}</p>
          <SessionForkButton
            sessionId={sessionId}
            sessionTitle={session?.title ?? null}
            sessionTags={session?.tags ?? []}
            className="mt-4"
          />
        </div>
      }
    />
  );
}
