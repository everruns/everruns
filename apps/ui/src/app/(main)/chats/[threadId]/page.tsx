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
import { usePageTitle } from "@/hooks";
import { useChatThreads } from "@/hooks/use-chat-threads";
import {
  isChatThread,
  threadTitle,
  PLATFORM_CHAT_STARTER_TAG,
  PLATFORM_CHAT_AGENT_NAME,
} from "@/lib/chat-threads";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { resolvePlatformChatIntro } from "@/lib/platform-chat-intro";

function ThreadContent({ threadId }: { threadId: string }) {
  const { session, agent, agentId, sessionLoading } = useSessionContext();
  // `GET /v1/sessions/{id}` carries no message preview — only the list does — so
  // an untitled thread takes its display title from the same list the sidebar
  // reads. Both surfaces then name the thread identically. Archived threads are
  // included: opening one by URL must still name it properly.
  const { threads } = useChatThreads({ includeArchived: true });
  const listEntry = threads.find((candidate) => candidate.id === threadId);
  const permanent = session?.tags.includes(PLATFORM_CHAT_STARTER_TAG) ?? false;
  const title = permanent
    ? "Chat"
    : threadTitle({
        title: session?.title ?? null,
        preview: session?.preview ?? listEntry?.preview ?? null,
      });

  const counterpart = getDisplayName(agent);

  const platformIntro = agent ? resolvePlatformChatIntro(agent) : null;

  usePageTitle(session ? title : null, "Chat");

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
        backLabel="Back to Chat"
        resourceId={threadId}
      />
    );
  }

  if (!isChatThread(session) || !agent || agent.name !== PLATFORM_CHAT_AGENT_NAME) {
    return (
      <ResourceNotFound
        title="Thread not found"
        description="This session is available as a recording. Use Playground to test its Agent."
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
        platformIntro={platformIntro?.intro ?? null}
        platformDescription={platformIntro?.description ?? null}
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
      <ChatPanel
        replyToLabel={counterpart}
        showRunCards
        showParticipants={false}
        platformIcon="everruns"
        platformIntro={platformIntro?.intro ?? null}
        platformStarters={platformIntro?.starters ?? []}
      />
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
