"use client";

// Room recording: what happened in a room an app made. Who posted, who woke
// and why, its shared files and its members. Read-only: the app that made the
// room owns its people-facing UI (knowledge/runtime-resources/rooms.md).

import { use, useState } from "react";
import { RoomIcon } from "@/components/icons/facet-icons";
import {
  PageBreadcrumb,
  PageContainer,
  PageMasthead,
  SectionTabs,
} from "@/components/layout/page-layout";
import {
  RoomActivityTable,
  RoomApiSnippet,
  RoomMembersTable,
} from "@/components/rooms/room-activity";
import { RoomFilesPanel } from "@/components/rooms/room-files-panel";
import { RoomMessageFeed } from "@/components/rooms/room-message-feed";
import { RoomSourceBadge } from "@/components/rooms/room-parts";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Badge } from "@/components/ui/badge";
import { LinkButton } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { usePageTitle } from "@/hooks";
import { useRoom } from "@/hooks/use-rooms";
import { isPlaygroundRoom, roomTotals } from "@/lib/rooms";
import { useUiTranslations } from "@/providers/locale-provider";

type Tab = "activity" | "messages" | "files" | "members" | "api";

export default function RoomRecordingPage({ params }: { params: Promise<{ roomId: string }> }) {
  const { roomId } = use(params);
  const ui = useUiTranslations();
  const [tab, setTab] = useState<Tab>("activity");
  const { room, members, messages, events } = useRoom(roomId);
  usePageTitle(room.data?.name ?? ui("Room"));

  if (room.error)
    return (
      <ResourceNotFound
        title={ui("Room not found")}
        description={ui("It may be in another organization, or the link is wrong.")}
        backHref="/rooms"
        backLabel={ui("Rooms")}
        resourceId={roomId}
      />
    );
  if (!room.data) {
    return (
      <PageContainer>
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-64 w-full" />
      </PageContainer>
    );
  }
  const current = room.data;
  const memberList = members.data ?? [];
  const eventList = events.data ?? [];
  const totals = roomTotals(eventList, memberList);
  const loading = members.isLoading || messages.isLoading || events.isLoading;

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Rooms"), href: "/rooms" }, { label: current.name }]} />
      <PageMasthead
        icon={<RoomIcon />}
        title={current.name}
        badges={
          <>
            <RoomSourceBadge room={current} />
            {current.archived && <Badge variant="outline">{ui("Archived")}</Badge>}
          </>
        }
        description={[
          current.topic,
          `${totals.messages} ${ui("messages")}`,
          `${totals.agentMessages} ${ui("from agents")}`,
          `${totals.wakes} ${ui("agent wakes")}`,
          totals.limited ? `${totals.limited} ${ui("stopped by limits")}` : null,
        ]
          .filter(Boolean)
          .join(" · ")}
        actions={
          isPlaygroundRoom(current) ? (
            <LinkButton href={`/playground/rooms/${current.id}`} variant="outline">
              {ui("Open in the Playground")}
            </LinkButton>
          ) : undefined
        }
      />
      <SectionTabs
        value={tab}
        onValueChange={(value) => setTab(value as Tab)}
        items={[
          { value: "activity", label: ui("Activity"), count: eventList.length || undefined },
          { value: "messages", label: ui("Messages") },
          { value: "files", label: ui("Files") },
          {
            value: "members",
            label: ui("Members"),
            count: memberList.filter((m) => !m.left).length || undefined,
          },
          { value: "api", label: ui("API") },
        ]}
      />
      {loading ? (
        <Skeleton className="h-64 w-full" />
      ) : tab === "activity" ? (
        <div className="border bg-card">
          <RoomActivityTable events={eventList} members={memberList} />
        </div>
      ) : tab === "messages" ? (
        <RoomMessageFeed
          room={current}
          messages={messages.data ?? []}
          members={memberList}
          events={eventList}
          sessionHref={(sessionId) => `/sessions/${sessionId}`}
        />
      ) : tab === "files" ? (
        <RoomFilesPanel roomId={current.id} />
      ) : tab === "members" ? (
        <div className="border bg-card">
          <RoomMembersTable
            members={memberList}
            events={eventList}
            playground={isPlaygroundRoom(current)}
          />
        </div>
      ) : (
        <div className="flex max-w-3xl flex-col gap-3">
          <p className="text-[13px] text-muted-foreground">
            {ui(
              "Your app follows this room through its change feed and posts through the same API the console reads.",
            )}
          </p>
          <RoomApiSnippet room={current} lastSeq={eventList.at(-1)?.seq} />
        </div>
      )}
    </PageContainer>
  );
}
