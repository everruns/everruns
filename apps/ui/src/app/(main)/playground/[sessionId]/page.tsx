"use client";

import { useLocale as useDisplayLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { use, useState } from "react";
import Link from "next/link";
import { Plus, Users } from "lucide-react";
import {
  SessionProvider,
  useSessionContext,
} from "@/app/(main)/sessions/[sessionId]/session-context";
import { ChatPanel } from "@/components/chat/chat-panel";
import { ChatThreadHeader } from "@/components/chat/chat-thread-header";
import { SessionTranscript } from "@/components/session/session-transcript";
import { SessionWorkspace } from "@/components/session/session-workspace";
import { ResourceNotFound } from "@/components/resource-not-found";
import { LinkButton } from "@/components/ui/button";
import {
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageControlStrip,
  PageMain,
  PageRail,
  RailSection,
  SectionTabs,
} from "@/components/layout/page-layout";
import { Skeleton } from "@/components/ui/skeleton";
import { useHarnesses, usePageTitle } from "@/hooks";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { useOrg } from "@/providers/org-provider";
import { getEventData } from "@/lib/api/types";
import { threadTitle } from "@/lib/chat-threads";
import { getDisplayName } from "@/lib/entity-lifecycle";

function Conversation({ id }: { id: string }) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const { session, agent, sessionLoading, chatEvents, getMessageText } = useSessionContext();
  const { data: harnesses = [] } = useHarnesses();
  const { data: subject, isLoading: subjectLoading } = useVirtualUser(
    session?.playground_user_id ?? undefined,
  );
  const { data: me } = useVirtualUser("me");
  const { hasRole } = useOrg();
  const [tab, setTab] = useState<"chat" | "workspace">("chat");
  useChatPageFilters({ tab: tab });
  // Session detail omits the list preview; use the same first user input for untitled chats.
  const firstInput = chatEvents
    .map((event) => getEventData(event, "input.message"))
    .find((data) => data?.message.role === "user");
  const title = threadTitle(
    {
      title: session?.title ?? null,
      preview: session?.preview ?? (firstInput ? getMessageText(firstInput) : null),
    },
    "New Playground chat",
  );
  const harness = harnesses.find((h) => h.id === session?.harness_id);
  const counterpart = agent
    ? getDisplayName(agent, displayLocale)
    : harness
      ? getDisplayName(harness, displayLocale)
      : undefined;
  const canSend = hasRole("admin") || (!!me && session?.playground_user_id === me.id);
  usePageTitle(title, ui("Playground"));
  if (sessionLoading) return <Skeleton className="m-6 h-64" />;
  if (!session || session.source !== "playground")
    return (
      <ResourceNotFound
        title={ui("Playground chat not found")}
        description={ui("This Playground chat is unavailable in your organisation.")}
        backHref="/playground"
        backLabel="Back to Playground"
        resourceId={id}
      />
    );
  return (
    <PageContainer fullWidth className="min-h-full xl:h-full xl:min-h-0">
      <PageBreadcrumb
        items={[{ label: ui("Playground"), href: "/playground" }, { label: title }]}
      />
      <ChatThreadHeader
        session={session}
        title={title}
        counterpart={counterpart}
        counterpartHref={
          agent ? (
            <Link
              href={`/agents/${agent.id}`}
              className="underline underline-offset-4 hover:text-primary"
            >
              {counterpart}
            </Link>
          ) : harness ? (
            <Link
              href={`/harnesses/${harness.id}`}
              className="underline underline-offset-4 hover:text-primary"
            >
              {counterpart}
            </Link>
          ) : undefined
        }
        contextLabel="Organisation shared"
        layout="page"
        showPin={false}
        extraActions={
          <LinkButton href="/playground/new" variant="accent">
            <Plus className="size-4" />
            {ui("New Playground chat")}
          </LinkButton>
        }
      />
      <PageControlStrip>
        <SectionTabs
          value={tab}
          onValueChange={(value) => setTab(value as "chat" | "workspace")}
          items={[
            { value: "chat", label: ui("Chat") },
            { value: "workspace", label: ui("Workspace") },
          ]}
        />
      </PageControlStrip>
      <PageColumns className="xl:min-h-0 xl:flex-1 xl:grid-cols-[minmax(0,1fr)_280px]">
        <PageMain className="h-[28rem] min-h-0 overflow-hidden border bg-background xl:h-auto">
          {tab === "workspace" ? (
            <SessionWorkspace />
          ) : subjectLoading ? (
            <Skeleton className="m-6 h-64" />
          ) : canSend && subject?.status === "active" && !session.archived_at ? (
            <ChatPanel
              replyToLabel={counterpart}
              showRunCards
              collapseWorkLog={false}
              showParticipants={false}
              platformIntro={agent?.intro_markdown}
              platformStarters={agent?.starters ?? []}
            />
          ) : (
            <>
              <SessionTranscript showRunCards />
              <p className="border-t bg-background p-4 text-center text-sm text-muted-foreground">
                {session.archived_at
                  ? ui("Unarchive this chat to continue.")
                  : subject?.status !== "active"
                    ? ui("The selected virtual user is unavailable.")
                    : ui(
                        "An organisation admin can continue Playground chats as another virtual user.",
                      )}
              </p>
            </>
          )}
        </PageMain>
        <PageRail>
          <RailSection label={agent ? ui("Agent") : ui("Harness")}>
            <Link
              href={agent ? `/agents/${agent.id}` : `/harnesses/${session.harness_id}`}
              className="text-sm font-medium underline underline-offset-4 hover:text-primary"
            >
              {counterpart ?? ui("Harness chat")}
            </Link>
          </RailSection>
          <RailSection label={ui("Talk as")}>
            <Link
              href={`/virtual-users/${session.playground_user_id}`}
              className="text-sm font-medium underline underline-offset-4 hover:text-primary"
            >
              {subject?.name ?? session.playground_user_id}
            </Link>
            <p className="mt-2 text-xs text-muted-foreground">{ui("Fixed for this chat")}</p>
          </RailSection>
          <RailSection label={ui("Sharing")}>
            <p className="flex items-center gap-2 text-sm font-medium">
              <Users className="size-4" /> {ui("Organisation shared")}
            </p>
            <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
              {ui("Private user connections and memory are unavailable here.")}
            </p>
          </RailSection>
        </PageRail>
      </PageColumns>
    </PageContainer>
  );
}
export default function PlaygroundChatPage({ params }: { params: Promise<{ sessionId: string }> }) {
  const { sessionId } = use(params);
  const { currentOrg } = useOrg();
  return (
    <SessionProvider key={`${currentOrg?.public_id}:${sessionId}`} sessionId={sessionId}>
      <Conversation id={sessionId} />
    </SessionProvider>
  );
}
