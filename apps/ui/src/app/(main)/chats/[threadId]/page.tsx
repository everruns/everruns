/**
 * The thread surface: header, transcript, composer.
 *
 * A thread is an ordinary session, so `SessionProvider` + `ChatPanel` carry the
 * transcript unchanged. Keying the provider on the thread id is what keeps
 * transcript state from leaking between threads — switching threads remounts the
 * whole subtree rather than re-pointing a live one at new events.
 *
 * Session detail is a read-only recording (EVE-854); this is where its "Fork
 * into chat" escape hatch lands.
 */
"use client";

import { use } from "react";
import Link from "next/link";
import { Bot } from "lucide-react";
import {
  SessionProvider,
  useSessionContext,
} from "@/app/(main)/sessions/[sessionId]/session-context";
import { ChatPanel } from "@/components/chat/chat-panel";
import { ChatThreadHeader } from "@/components/chat/chat-thread-header";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Skeleton } from "@/components/ui/skeleton";
import { useHarnesses, usePageTitle } from "@/hooks";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { isChatThread, threadTitle } from "@/lib/chat-threads";
import { getDisplayName } from "@/lib/entity-lifecycle";

function ThreadContent({ threadId }: { threadId: string }) {
  const { session, agent, agentId, sessionLoading } = useSessionContext();
  const { data: harnesses } = useHarnesses();
  // `GET /v1/sessions/{id}` carries no message preview — only the list does — so
  // an untitled thread takes its display title from the same list the sidebar
  // reads. Both surfaces then name the thread identically.
  const { threads } = useChatThreads();
  const listEntry = threads.find((candidate) => candidate.id === threadId);
  const title = threadTitle({
    title: session?.title ?? null,
    preview: session?.preview ?? listEntry?.preview ?? null,
  });

  // A Platform Chat thread is bound to a harness rather than an agent; name that
  // harness so the binding still reads as a fact instead of a blank.
  const harness = session?.harness_id
    ? harnesses?.find((candidate) => candidate.id === session.harness_id)
    : undefined;
  const counterpart = agentId
    ? getDisplayName(agent)
    : harness
      ? getDisplayName(harness)
      : undefined;

  usePageTitle(session ? title : null, "Chats");

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
        backHref="/chats"
        backLabel="Back to chats"
        resourceId={threadId}
      />
    );
  }

  if (!isChatThread(session)) {
    return (
      <ResourceNotFound
        title="Thread not found"
        description="This session is a read-only recording. Fork it into a chat before continuing the conversation."
        backHref={`/sessions/${threadId}/transcript`}
        backLabel="Open recording"
        resourceId={threadId}
      />
    );
  }

  return (
    <div className="flex h-full flex-col bg-background bg-brand-dots">
      <ChatThreadHeader
        session={session}
        title={title}
        counterpart={counterpart}
        counterpartHref={
          agentId ? (
            <Link
              href={`/agents/${agentId}`}
              className="inline-flex items-center gap-1 hover:text-foreground"
            >
              <Bot className="icon-sharp size-3" />
              {counterpart}
            </Link>
          ) : undefined
        }
      />
      <ChatPanel replyToLabel={counterpart} showRunCards />
    </div>
  );
}

function ThreadRoute({ params }: { params: Promise<{ threadId: string }> }) {
  const { threadId } = use(params);

  return (
    <SessionProvider key={threadId} sessionId={threadId}>
      <ThreadContent threadId={threadId} />
    </SessionProvider>
  );
}

export default function ChatThreadPage({ params }: { params: Promise<{ threadId: string }> }) {
  return <ThreadRoute params={params} />;
}
