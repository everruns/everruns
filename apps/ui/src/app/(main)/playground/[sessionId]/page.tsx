"use client";

import { use, useState } from "react";
import { Users } from "lucide-react";
import {
  SessionProvider,
  useSessionContext,
} from "@/app/(main)/sessions/[sessionId]/session-context";
import { ChatPanel } from "@/components/chat/chat-panel";
import { ChatThreadHeader } from "@/components/chat/chat-thread-header";
import { SessionTranscript } from "@/components/session/session-transcript";
import { SessionWorkspace } from "@/components/session/session-workspace";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Button, LinkButton } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useHarnesses, usePageTitle } from "@/hooks";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { useOrg } from "@/providers/org-provider";
import { threadTitle } from "@/lib/chat-threads";
import { getDisplayName } from "@/lib/entity-lifecycle";

function Conversation({ id }: { id: string }) {
  const { session, agent, sessionLoading } = useSessionContext();
  const { data: harnesses = [] } = useHarnesses();
  const { data: subject, isLoading: subjectLoading } = useVirtualUser(
    session?.playground_user_id ?? undefined,
  );
  const { data: me } = useVirtualUser("me");
  const { hasRole } = useOrg();
  const [tab, setTab] = useState<"conversation" | "workspace">("conversation");
  const title = session ? threadTitle(session, "New conversation") : "Conversation";
  const harness = harnesses.find((h) => h.id === session?.harness_id);
  const counterpart = agent ? getDisplayName(agent) : harness ? getDisplayName(harness) : undefined;
  const canSend = hasRole("admin") || (!!me && session?.playground_user_id === me.id);
  usePageTitle(title, "Playground");
  if (sessionLoading) return <Skeleton className="m-6 h-64" />;
  if (!session || session.source !== "playground")
    return (
      <ResourceNotFound
        title="Conversation not found"
        description="This Playground conversation is unavailable in your organisation."
        backHref="/playground"
        backLabel="Back to Playground"
        resourceId={id}
      />
    );
  return (
    <div className="flex h-full flex-col bg-background bg-brand-dots">
      <ChatThreadHeader
        session={session}
        title={title}
        counterpart={counterpart}
        contextLabel="Organisation shared"
      />
      <div className="flex flex-wrap items-center gap-2 border-b bg-background/80 px-6 py-2">
        <Button
          size="sm"
          variant={tab === "conversation" ? "secondary" : "ghost"}
          onClick={() => setTab("conversation")}
        >
          Conversation
        </Button>
        <Button
          size="sm"
          variant={tab === "workspace" ? "secondary" : "ghost"}
          onClick={() => setTab("workspace")}
        >
          Workspace
        </Button>
        <LinkButton href={`/sessions/${id}/timeline`} variant="ghost" size="sm">
          Trace
        </LinkButton>
        <LinkButton href="/playground/new" variant="ghost" size="sm" className="ml-auto">
          New conversation
        </LinkButton>
      </div>
      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        <main className="flex min-h-0 flex-1 flex-col">
          {tab === "workspace" ? (
            <SessionWorkspace />
          ) : subjectLoading ? (
            <Skeleton className="m-6 h-64" />
          ) : canSend && subject?.status === "active" && !session.archived_at ? (
            <ChatPanel replyToLabel={counterpart} showRunCards showParticipants={false} />
          ) : (
            <>
              <SessionTranscript showRunCards />
              <p className="border-t bg-background p-4 text-center text-sm text-muted-foreground">
                {session.archived_at
                  ? "Unarchive this conversation to continue."
                  : subject?.status !== "active"
                    ? "The selected virtual user is unavailable."
                    : "An organisation admin can continue conversations as another virtual user."}
              </p>
            </>
          )}
        </main>
        <aside className="space-y-6 border-t bg-background/90 p-6 lg:w-72 lg:border-l lg:border-t-0">
          <div>
            <p className="text-xs uppercase tracking-widest text-muted-foreground">Agent</p>
            <p className="mt-2 text-sm font-medium">{counterpart ?? "Harness conversation"}</p>
          </div>
          <div>
            <p className="text-xs uppercase tracking-widest text-muted-foreground">Talk as</p>
            <p className="mt-2 text-sm font-medium">
              {subject?.name ?? session.playground_user_id}
            </p>
            <p className="mt-2 text-xs text-muted-foreground">Fixed for this conversation</p>
          </div>
          <div className="border-t pt-4">
            <p className="flex items-center gap-2 text-sm font-medium">
              <Users className="size-4" />
              Organisation shared
            </p>
            <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
              Private user connections and memory are unavailable here.
            </p>
          </div>
        </aside>
      </div>
    </div>
  );
}

export default function PlaygroundConversationPage({
  params,
}: {
  params: Promise<{ sessionId: string }>;
}) {
  const { sessionId } = use(params);
  const { currentOrg } = useOrg();
  return (
    <SessionProvider key={`${currentOrg?.public_id}:${sessionId}`} sessionId={sessionId}>
      <Conversation id={sessionId} />
    </SessionProvider>
  );
}
