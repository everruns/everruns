"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { GitFork, Loader2 } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import Link from "next/link";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useForkSession } from "@/hooks/use-sessions";
import { CHAT_THREAD_TAG, PLATFORM_CHAT_STARTER_TAG } from "@/lib/chat-threads";
import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

export function SessionForkButton({
  sessionId,
  sessionTitle,
  sessionTags,
  className,
  agentId,
  platformChat = false,
}: {
  sessionId: string;
  sessionTitle: string | null;
  sessionTags: string[];
  className?: string;
  agentId?: string;
  platformChat?: boolean;
}) {
  const router = useRouter();
  const { t } = useLocale();
  const forkSession = useForkSession();
  const playgroundEnabled = useFeatureFlag("playground");
  const [forkError, setForkError] = useState<string | null>(null);

  const handleFork = () => {
    setForkError(null);
    forkSession.mutate(
      {
        sessionId,
        request: {
          title: sessionTitle ? `${sessionTitle} (chat)` : undefined,
          // Chat routes accept only marked sessions, so every recording fork
          // must carry the thread tag even when the recording already has tags.
          tags: Array.from(
            new Set([
              ...sessionTags.filter((t) => t !== PLATFORM_CHAT_STARTER_TAG),
              CHAT_THREAD_TAG,
            ]),
          ),
        },
      },
      {
        onSuccess: (session) => router.push(`/chats/${session.id}`),
        onError: (error) =>
          setForkError(error instanceof Error ? error.message : t("fork_session_error")),
      },
    );
  };

  if (!platformChat) {
    return playgroundEnabled ? (
      <Link
        className={buttonVariants({ variant: "outline", size: "sm", className })}
        href={`/playground/new${agentId ? `?agent=${encodeURIComponent(agentId)}` : ""}`}
      >
        Test in Playground
      </Link>
    ) : null;
  }

  return (
    <>
      <button
        type="button"
        onClick={handleFork}
        disabled={forkSession.isPending}
        className={cn(buttonVariants({ variant: "default", size: "sm" }), "gap-1", className)}
      >
        {forkSession.isPending ? (
          <Loader2 className="icon-sharp h-4 w-4 animate-spin" />
        ) : (
          <GitFork className="icon-sharp h-4 w-4" />
        )}
        {t("fork_into_chat")}
      </button>

      {forkError && (
        <span role="alert" className="text-xs text-destructive">
          {forkError}
        </span>
      )}
    </>
  );
}
