"use client";

// Playground room: a test bench for agents working together. Post as yourself
// or a test user; under each message, who woke and why; beside it each
// agent's session, the deliveries and the room's limits. It is the same room
// API an app calls, so what works here works from code.

import Link from "next/link";
import { use, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageMain,
  PageRail,
  RailSection,
  SectionTabs,
} from "@/components/layout/page-layout";
import { RoomApiSnippet } from "@/components/rooms/room-activity";
import { RoomComposer } from "@/components/rooms/room-composer";
import { RoomFilesPanel } from "@/components/rooms/room-files-panel";
import { RoomMessageFeed } from "@/components/rooms/room-message-feed";
import { MemberTile, useMemberKindLabel, useWakeModeLabel } from "@/components/rooms/room-parts";
import { ResourceNotFound } from "@/components/resource-not-found";
import { LinkButton } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { useCurrentUser, usePageTitle } from "@/hooks";
import { roomKeys, useRoom } from "@/hooks/use-rooms";
import { updateRoomMember, type RoomMessage, type WakeMode } from "@/lib/api/rooms";
import { formatDate } from "@/lib/formatting";
import { roomTotals, sameUserId, sessionsByMember } from "@/lib/rooms";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";

// Agents answer a few seconds after a post; the bench reads on while open.
const POLL_MS = 3000;

export default function PlaygroundRoomPage({ params }: { params: Promise<{ roomId: string }> }) {
  const { roomId } = use(params);
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const { hasRole } = useOrg();
  const { data: currentUser } = useCurrentUser();
  const queryClient = useQueryClient();
  const kindLabel = useMemberKindLabel();
  const wakeLabel = useWakeModeLabel();
  const [tab, setTab] = useState<"messages" | "files">("messages");
  const [replyTo, setReplyTo] = useState<RoomMessage | undefined>();
  const { room, members, messages, events } = useRoom(roomId, { poll: POLL_MS });
  usePageTitle(room.data?.name ?? ui("Playground room"));
  const changeWake = useMutation({
    mutationFn: ({ memberId, wake }: { memberId: string; wake: WakeMode }) =>
      updateRoomMember(roomId, memberId, { wake }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: roomKeys.room(roomId) }),
  });

  if (room.error) {
    return (
      <ResourceNotFound
        title={ui("Room not found")}
        description={ui("It may be in another organization, or the link is wrong.")}
        backHref="/playground/rooms"
        backLabel={ui("Playground rooms")}
        resourceId={roomId}
      />
    );
  }
  if (!room.data || !members.data || !messages.data || !events.data) {
    return (
      <PageContainer>
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-96 w-full" />
      </PageContainer>
    );
  }

  const current = room.data;
  const memberList = members.data;
  const eventList = events.data;
  const you = memberList.find(
    (member) => member.kind === "person" && sameUserId(member.user_id, currentUser?.id),
  );
  const canPostForOthers = hasRole("admin");
  const sessions = sessionsByMember(eventList);
  const totals = roomTotals(eventList, memberList);
  const memberById = new Map(memberList.map((member) => [member.id, member]));
  const deliveries = eventList
    .filter((event) => event.type === "agent.woken" || event.type === "agent.limited")
    .slice(-8)
    .reverse();
  const sessionHref = (sessionId: string) => `/sessions/${sessionId}`;

  return (
    <PageContainer fullWidth>
      <PageBreadcrumb
        items={[
          { label: ui("Playground"), href: "/playground" },
          { label: ui("Rooms"), href: "/playground/rooms" },
          { label: current.name },
        ]}
      />
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-xl font-semibold">{current.name}</h1>
        <LinkButton href={`/rooms/${current.id}`} variant="outline">
          {ui("Open recording")}
        </LinkButton>
      </div>
      <PageColumns>
        <PageMain className="flex min-w-0 flex-col gap-4">
          <SectionTabs
            value={tab}
            onValueChange={(value) => setTab(value as "messages" | "files")}
            items={[
              { value: "messages", label: ui("Messages") },
              { value: "files", label: ui("Files") },
            ]}
          />
          {tab === "messages" ? (
            <>
              <RoomMessageFeed
                room={current}
                messages={messages.data}
                members={memberList}
                events={eventList}
                playground
                sessionHref={sessionHref}
                onReply={setReplyTo}
                refetchInterval={POLL_MS}
              />
              <RoomComposer
                room={current}
                members={memberList}
                you={you}
                canPostForOthers={canPostForOthers}
                replyTo={replyTo}
                onCancelReply={() => setReplyTo(undefined)}
              />
            </>
          ) : (
            <RoomFilesPanel roomId={current.id} refetchInterval={POLL_MS} />
          )}
        </PageMain>
        <PageRail>
          <RailSection label={ui("Members")}>
            <ul className="flex flex-col gap-3">
              {memberList
                .filter((member) => !member.left)
                .map((member) => {
                  const memberSessions = sessions.get(member.id) ?? [];
                  return (
                    <li key={member.id} className="flex items-start gap-2.5">
                      <MemberTile
                        name={member.display_name}
                        kind={member.kind}
                        className="size-7 text-xs"
                      />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[13px] font-medium">
                          {member.display_name}
                        </div>
                        <div className="font-mono text-[11px] text-muted-foreground">
                          @{member.handle} · {kindLabel(member.kind, true)}
                        </div>
                        {member.kind === "agent" && (
                          <div className="mt-1 flex flex-wrap items-center gap-2">
                            <Select
                              value={member.wake}
                              onValueChange={(wake) =>
                                changeWake.mutate({ memberId: member.id, wake: wake as WakeMode })
                              }
                            >
                              <SelectTrigger
                                size="sm"
                                aria-label={`${ui("Wakes")}: ${member.display_name}`}
                                className="h-6 w-auto text-[11px]"
                              >
                                <SelectValue />
                              </SelectTrigger>
                              <SelectContent>
                                {(["mentions", "relevant", "everything", "never"] as const).map(
                                  (wake) => (
                                    <SelectItem key={wake} value={wake}>
                                      {wakeLabel(wake)}
                                    </SelectItem>
                                  ),
                                )}
                              </SelectContent>
                            </Select>
                            <span className="flex items-center gap-2">
                              {memberSessions.map((sessionId, index) => (
                                <Link
                                  key={sessionId}
                                  href={sessionHref(sessionId)}
                                  className="text-[11px] text-primary hover:underline"
                                >
                                  {ui("Session")} {memberSessions.length > 1 ? index + 1 : ""}
                                </Link>
                              ))}
                            </span>
                          </div>
                        )}
                      </div>
                    </li>
                  );
                })}
            </ul>
          </RailSection>
          <RailSection label={ui("Deliveries")}>
            {deliveries.length === 0 ? (
              <p className="text-[13px] text-muted-foreground">{ui("No agent has woken yet.")}</p>
            ) : (
              <ul className="flex flex-col gap-1.5 text-[13px]">
                {deliveries.map((event) => (
                  <li key={event.seq} className="flex gap-2">
                    <span className="shrink-0 text-muted-foreground tabular-nums">
                      {new Date(event.created_at).toLocaleTimeString(displayLocale, {
                        hour: "2-digit",
                        minute: "2-digit",
                      })}
                    </span>
                    <span className="min-w-0">
                      {memberById.get(event.member_id ?? "")?.display_name ?? event.member_id}{" "}
                      {event.type === "agent.woken" ? ui("woke") : ui("was stopped by a limit")}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </RailSection>
          <RailSection label={ui("This room")}>
            <p className="text-[13px] text-muted-foreground">
              {totals.messages} {ui("messages")} · {totals.agentMessages} {ui("from agents")} ·{" "}
              {totals.wakes} {ui("agent wakes")}
            </p>
            <p className="mt-1 text-[13px] text-muted-foreground">
              {ui("Limits")}: {current.loop_limits.max_hops} {ui("agent replies in a row")} ·{" "}
              {current.loop_limits.max_agent_posts_hour} {ui("agent messages an hour")}
            </p>
            <p className="mt-1 text-[11px] text-muted-foreground">
              {ui("Created")} {formatDate(current.created_at, displayLocale)}
            </p>
          </RailSection>
          <RailSection label={ui("Same thing from your app")}>
            <RoomApiSnippet room={current} lastSeq={eventList.at(-1)?.seq} />
          </RailSection>
        </PageRail>
      </PageColumns>
    </PageContainer>
  );
}
