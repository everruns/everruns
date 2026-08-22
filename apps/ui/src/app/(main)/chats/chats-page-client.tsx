/**
 * All chats. Also the app's landing route, so the no-threads case has to be a
 * usable starting point rather than a spinner: the empty state is the new-chat
 * form itself.
 */
"use client";

import { useState } from "react";
import Link from "next/link";
import { MessageCircle, Plus } from "lucide-react";
import { AgentAvatar } from "@/components/chat/agent-avatar";
import { ChatArchiveButton } from "@/components/chat/chat-archive-button";
import { ChatPinButton } from "@/components/chat/chat-pin-button";
import { NewChatForm } from "@/components/chat/new-chat-form";
import { EmptyState, PageContainer, PageMasthead } from "@/components/layout";
import { buttonVariants } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { ArchiveFilter } from "@/components/archive-filter";
import { useAgents, useHarnesses } from "@/hooks";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { isArchivedThread, threadTitle } from "@/lib/chat-threads";
import { formatRelativeTime } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import { getDisplayName } from "@/lib/entity-lifecycle";
import type { Session } from "@/lib/api/types";

function ThreadRow({ thread, counterpart }: { thread: Session; counterpart?: string }) {
  const archived = isArchivedThread(thread);
  return (
    <div
      className={cn(
        "flex items-center border border-border/70 bg-card/80 transition-colors hover:bg-muted/40",
        // Archived rows only appear when the user asked for them, so they are
        // dimmed rather than hidden — recognisable, but visibly set aside.
        archived && "opacity-60",
      )}
    >
      <Link
        href={`/chats/${thread.id}`}
        className="flex min-w-0 flex-1 items-center gap-3 px-4 py-3"
      >
        <AgentAvatar name={counterpart} size="sm" />
        <div className="flex min-w-0 flex-col">
          <span className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-medium text-foreground">
              {threadTitle(thread)}
            </span>
            {archived && (
              <span className="flex-none border border-border/70 px-1.5 text-[11px] uppercase tracking-wide text-muted-foreground">
                Archived
              </span>
            )}
          </span>
          {thread.output_preview && (
            <span className="truncate text-xs text-muted-foreground">{thread.output_preview}</span>
          )}
        </div>
        <span className="ml-auto flex-none text-xs text-muted-foreground">
          {formatRelativeTime(thread.updated_at)}
        </span>
      </Link>
      <div className="mr-3 flex items-center gap-1">
        <ChatPinButton session={thread} />
        <ChatArchiveButton session={thread} />
      </div>
    </div>
  );
}

export default function ChatsPageClient() {
  // Archived threads are hidden until asked for; the filter is always offered
  // so "show all chats" is one click from any state of this page, including
  // the empty one.
  const [showArchived, setShowArchived] = useState(false);
  const { threads, isLoading } = useChatThreads({ includeArchived: showArchived });
  const { data: agents = [] } = useAgents();
  const { data: harnesses = [] } = useHarnesses();
  // Hold the starting state open once the user commits, so the new thread landing
  // in the list cannot unmount the form mid-navigation.
  const [starting, setStarting] = useState(false);
  // Same counterpart rule as the thread header: the agent when there is one,
  // otherwise the harness the thread is bound to.
  const agentNameById = new Map(agents.map((agent) => [agent.id, getDisplayName(agent)]));
  const harnessNameById = new Map(
    harnesses.map((harness) => [harness.id, getDisplayName(harness)]),
  );
  const counterpartOf = (thread: Session) =>
    (thread.agent_id ? agentNameById.get(thread.agent_id) : undefined) ??
    harnessNameById.get(thread.harness_id);

  return (
    <PageContainer>
      <PageMasthead
        icon={<MessageCircle />}
        title="Chats"
        description="Conversations with agents and harnesses. Each thread is an ordinary session you can open, share, and re-read."
        actions={
          <>
            <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
            <Link href="/chats/new" className={buttonVariants()}>
              <Plus className="size-4" />
              New chat
            </Link>
          </>
        }
      />

      <div className="mt-6 space-y-2">
        {isLoading &&
          Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-[58px] w-full" />
          ))}

        {!isLoading && (threads.length === 0 || starting) && (
          <EmptyState
            icon={<MessageCircle />}
            title="No chats yet"
            description="Pick an agent or harness and start talking. The thread is saved as a session you can come back to."
            action={<NewChatForm onStartingChange={setStarting} />}
          />
        )}

        {!starting &&
          threads.map((thread) => (
            <ThreadRow key={thread.id} thread={thread} counterpart={counterpartOf(thread)} />
          ))}
      </div>
    </PageContainer>
  );
}
