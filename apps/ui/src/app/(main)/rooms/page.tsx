"use client";

// Rooms (Operational, next to Sessions): every room in the organization,
// whoever made it, for inspection. Apps own the people-facing room UI; the
// Playground is where a builder talks in one (knowledge/runtime-resources/rooms.md).

import Link from "next/link";
import { useMemo, useState } from "react";
import { useQueries } from "@tanstack/react-query";
import { RoomIcon } from "@/components/icons/facet-icons";
import {
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageMasthead,
  SectionTabs,
} from "@/components/layout/page-layout";
import { RoomSourceBadge } from "@/components/rooms/room-parts";
import { Badge } from "@/components/ui/badge";
import { LinkButton } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { usePageTitle } from "@/hooks";
import { useRooms } from "@/hooks/use-rooms";
import { listRoomMembers } from "@/lib/api/rooms";
import { formatRelativeTime } from "@/lib/formatting";
import { countMembers, isPlaygroundRoom, type RoomSource, roomSource } from "@/lib/rooms";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

type SourceFilter = "all" | RoomSource;

export default function RoomsPage() {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  usePageTitle(ui("Rooms"));
  const [search, setSearch] = useState("");
  const [source, setSource] = useState<SourceFilter>("all");
  const [archived, setArchived] = useState(false);
  const { data: allRooms, isLoading, error } = useRooms({ includeArchived: archived });

  const rooms = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return (allRooms ?? []).filter(
      (room) =>
        room.archived === archived &&
        (source === "all" || roomSource(room) === source) &&
        (!needle ||
          `${room.name} ${room.topic ?? ""} ${room.external_ref ?? ""} ${room.id}`
            .toLowerCase()
            .includes(needle)),
    );
  }, [allRooms, archived, search, source]);

  const memberQueries = useQueries({
    queries: rooms.slice(0, 100).map((room) => ({
      queryKey: ["rooms", room.id, "members"],
      queryFn: () => listRoomMembers(room.id),
      staleTime: 30_000,
    })),
  });

  const membersLabel = (index: number) => {
    const members = memberQueries[index]?.data;
    if (!members) return "…";
    const counts = countMembers(members);
    const parts: string[] = [];
    if (counts.agents)
      parts.push(`${counts.agents} ${counts.agents === 1 ? ui("agent") : ui("agents")}`);
    if (counts.people)
      parts.push(`${counts.people} ${counts.people === 1 ? ui("person") : ui("people")}`);
    if (counts.outsideAgents)
      parts.push(
        `${counts.outsideAgents} ${counts.outsideAgents === 1 ? ui("outside agent") : ui("outside agents")}`,
      );
    if (counts.guests)
      parts.push(`${counts.guests} ${counts.guests === 1 ? ui("guest") : ui("guests")}`);
    return parts.join(" · ") || "—";
  };

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Rooms") }]} />
      <PageMasthead
        icon={<RoomIcon />}
        title={ui("Rooms")}
        badges={allRooms ? <Badge variant="outline">{rooms.length}</Badge> : undefined}
        description={ui(
          "Every room in the organization, whoever made it: your apps through the API, MCP clients and the Playground. Open one to see who posted, who woke and why.",
        )}
        actions={
          <LinkButton href="/playground/rooms" variant="outline">
            {ui("Try a room in the Playground")}
          </LinkButton>
        }
      />
      <PageControlStrip className="flex flex-wrap items-center gap-2">
        <SearchInput
          containerClassName="w-64"
          aria-label={ui("Search rooms")}
          placeholder={ui("Search rooms…")}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span>{ui("Made by")}</span>
          <SegmentedControl
            label={ui("Made by")}
            value={source}
            onValueChange={(value) => setSource(value as SourceFilter)}
            options={[
              { value: "all", label: ui("All") },
              { value: "api", label: ui("API") },
              { value: "playground", label: ui("Playground") },
            ]}
          />
        </div>
        <div className="flex-1" />
        <SectionTabs
          className="w-auto shrink-0 gap-3 border-b-0"
          value={archived ? "archived" : "active"}
          onValueChange={(value) => setArchived(value === "archived")}
          items={[
            { value: "active", label: ui("Active") },
            { value: "archived", label: ui("Archived") },
          ]}
        />
      </PageControlStrip>
      {error ? (
        <ChatErrorAlert message={ui("Could not load rooms.")} />
      ) : isLoading ? (
        <Skeleton className="h-56 w-full" />
      ) : rooms.length === 0 ? (
        <div className="flex flex-col items-center gap-2 border bg-card px-6 py-12 text-center">
          <RoomIcon className="size-5 text-muted-foreground" />
          <div className="text-sm font-semibold">
            {search || source !== "all"
              ? ui("No matching rooms")
              : archived
                ? ui("No archived rooms")
                : ui("No rooms yet")}
          </div>
          <p className="max-w-md text-[13px] text-muted-foreground">
            {ui(
              "Apps create rooms through the API, MCP clients through the room tools. Try one in the Playground to see how your agents work together.",
            )}
          </p>
        </div>
      ) : (
        <div className="border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{ui("Room")}</TableHead>
                <TableHead>{ui("Made by")}</TableHead>
                <TableHead>{ui("Members")}</TableHead>
                <TableHead className="text-right">{ui("Last activity")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rooms.map((room, index) => (
                <TableRow key={room.id}>
                  <TableCell>
                    <Link
                      href={
                        isPlaygroundRoom(room)
                          ? `/playground/rooms/${room.id}`
                          : `/rooms/${room.id}`
                      }
                      className="font-medium hover:underline"
                    >
                      {room.name}
                    </Link>
                    {room.topic && (
                      <div className="max-w-md truncate text-xs text-muted-foreground">
                        {room.topic}
                      </div>
                    )}
                  </TableCell>
                  <TableCell>
                    <span className="flex flex-wrap items-center gap-1.5">
                      <RoomSourceBadge room={room} />
                      {room.external_ref && (
                        <span className="font-mono text-[11px] text-muted-foreground">
                          {room.external_ref}
                        </span>
                      )}
                    </span>
                  </TableCell>
                  <TableCell className="text-[13px] text-muted-foreground">
                    {membersLabel(index)}
                  </TableCell>
                  <TableCell className="text-right text-[13px] text-muted-foreground">
                    {formatRelativeTime(room.last_message_at ?? room.updated_at, displayLocale)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </PageContainer>
  );
}
