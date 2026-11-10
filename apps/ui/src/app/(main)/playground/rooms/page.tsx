"use client";

// Playground rooms: test benches for agents working together. Each is an
// ordinary room marked `metadata.playground`, so it also shows under Rooms.

import Link from "next/link";
import { useState } from "react";
import { Plus, Users } from "lucide-react";
import { PlaygroundIcon, RoomIcon } from "@/components/icons/facet-icons";
import {
  PageBreadcrumb,
  PageContainer,
  PageFooter,
  PageMasthead,
} from "@/components/layout/page-layout";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { NewRoomDialog } from "@/components/rooms/new-room-dialog";
import { PlaygroundTabs } from "@/components/rooms/playground-tabs";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { usePageTitle } from "@/hooks";
import { useRooms } from "@/hooks/use-rooms";
import { formatRelativeTime } from "@/lib/formatting";
import { isPlaygroundRoom } from "@/lib/rooms";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";

export default function PlaygroundRoomsPage() {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const { currentOrg } = useOrg();
  usePageTitle(ui("Playground rooms"));
  const [creating, setCreating] = useState(false);
  const { data, isLoading, error } = useRooms();
  const rooms = (data ?? []).filter(isPlaygroundRoom);

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: ui("Playground"), href: "/playground" }, { label: ui("Rooms") }]}
      />
      <PageMasthead
        icon={<PlaygroundIcon />}
        title={ui("Playground")}
        description={ui(
          "Try several agents in one room: post as test users, see who wakes and why, and open each agent's session.",
        )}
        actions={
          <Button variant="accent" onClick={() => setCreating(true)}>
            <Plus className="size-4" />
            {ui("New room")}
          </Button>
        }
      />
      <PlaygroundTabs value="rooms" />
      {error ? (
        <ChatErrorAlert message={ui("Could not load rooms.")} />
      ) : isLoading ? (
        <Skeleton className="h-40 w-full" />
      ) : rooms.length === 0 ? (
        <div className="flex flex-col items-center gap-2 border bg-card px-6 py-12 text-center">
          <RoomIcon className="size-5 text-muted-foreground" />
          <div className="text-sm font-semibold">{ui("No Playground rooms yet")}</div>
          <p className="max-w-md text-[13px] text-muted-foreground">
            {ui(
              "Choose a few agents and test users to see how they work together before an app uses them.",
            )}
          </p>
          <Button className="mt-2" variant="outline" onClick={() => setCreating(true)}>
            <Plus className="size-4" />
            {ui("New room")}
          </Button>
        </div>
      ) : (
        <ul className="divide-y border bg-card">
          {rooms.map((room) => (
            <li key={room.id}>
              <Link
                href={`/playground/rooms/${room.id}`}
                className="flex items-center gap-3 px-4 py-3 hover:bg-muted"
              >
                <RoomIcon className="size-4 text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">{room.name}</span>
                  {room.topic && (
                    <span className="block truncate text-xs text-muted-foreground">
                      {room.topic}
                    </span>
                  )}
                </span>
                <span className="text-xs text-muted-foreground">
                  {formatRelativeTime(room.last_message_at ?? room.created_at, displayLocale)}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
      <PageFooter>
        <span className="flex items-center gap-2">
          <Users className="size-3.5" />
          {ui("Visible to everyone in")} {currentOrg?.name ?? ui("your organisation")}
        </span>
      </PageFooter>
      <NewRoomDialog open={creating} onOpenChange={setCreating} />
    </PageContainer>
  );
}
