"use client";

// A chat thread is an ordinary session rendered with a composer.
//
// Session detail is a read-only recording (EVE-854), so "Fork into chat" needs
// somewhere to land: this route. It is deliberately the thinnest possible
// thread surface — header plus `ChatPanel`. The sidebar thread list, the
// `/chats` index and the landing-route move are EVE-851's work and are not
// started here.

import { use } from "react";
import Link from "next/link";
import { Bot, MonitorPlay } from "lucide-react";
import { ChatPanel } from "@/components/chat/chat-panel";
import {
  SessionProvider,
  useSessionContext,
} from "@/app/(main)/sessions/[sessionId]/session-context";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Skeleton } from "@/components/ui/skeleton";
import { buttonVariants } from "@/components/ui/button";
import { usePageTitle } from "@/hooks";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { cn, shortenId } from "@/lib/utils";

function ThreadContent({ threadId }: { threadId: string }) {
  const { session, agent, agentId, sessionLoading } = useSessionContext();

  usePageTitle(session?.title ?? "Thread", "Chats");

  if (sessionLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="mb-4 h-8 w-1/3" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!session) {
    return (
      <ResourceNotFound
        title="Thread not found"
        description="This thread may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/sessions"
        backLabel="Back to sessions"
        resourceId={threadId}
      />
    );
  }

  return (
    <div className="flex h-full flex-col bg-background bg-brand-dots">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/70 bg-background/70 px-6 py-3.5 backdrop-blur-[1px]">
        <div className="min-w-0">
          <h1 className="truncate text-xl font-semibold tracking-tight">
            {session.title || `Thread ${shortenId(session.id)}`}
          </h1>
          {agentId && (
            <Link
              href={`/agents/${agentId}`}
              className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
            >
              <Bot className="icon-sharp h-3 w-3" />
              {getDisplayName(agent)}
            </Link>
          )}
        </div>

        <Link
          href={`/sessions/${threadId}`}
          className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1")}
        >
          <MonitorPlay className="icon-sharp h-4 w-4" />
          Open session
        </Link>
      </div>

      <ChatPanel />
    </div>
  );
}

export default function ChatThreadPage({ params }: { params: Promise<{ threadId: string }> }) {
  const { threadId } = use(params);

  return (
    <SessionProvider sessionId={threadId}>
      <ThreadContent threadId={threadId} />
    </SessionProvider>
  );
}
