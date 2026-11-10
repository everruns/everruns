"use client";

// The recording's views of a room beyond its messages: what happened, in
// order; who is in it; and how to follow the same thing from code.

import Link from "next/link";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CodeBlock } from "@/components/ui/code-block";
import type { Room, RoomEvent, RoomMember } from "@/lib/api/rooms";
import { formatDate } from "@/lib/formatting";
import { sessionsByMember } from "@/lib/rooms";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";
import { MemberTile, useMemberKindLabel, useWakeModeLabel } from "./room-parts";

export function RoomActivityTable({
  events,
  members,
}: {
  events: RoomEvent[];
  members: RoomMember[];
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const memberById = new Map(members.map((member) => [member.id, member]));
  const name = (id?: string | null) =>
    id ? (memberById.get(id)?.display_name ?? id) : ui("The room");
  const describe = (event: RoomEvent): string => {
    const data = (event.data ?? {}) as Record<string, unknown>;
    switch (event.type) {
      case "room.created":
        return ui("created the room");
      case "room.updated":
        return ui("changed the room's settings");
      case "room.archived":
        return ui("archived the room");
      case "room.unarchived":
        return ui("brought the room back");
      case "member.joined":
        return ui("joined");
      case "member.left":
        return ui("left");
      case "message.posted":
        return ui("posted a message");
      case "message.edited":
        return ui("edited a message");
      case "message.deleted":
        return ui("deleted a message");
      case "message.pinned":
        return ui("pinned a message");
      case "message.unpinned":
        return ui("unpinned a message");
      case "reaction.added":
        return `${ui("reacted")} ${typeof data.emoji === "string" ? data.emoji : ""}`.trim();
      case "reaction.removed":
        return ui("removed a reaction");
      case "agent.woken":
        return ui("woke for a message");
      case "agent.limited":
        return `${ui("was not woken: a loop limit stopped it")} (${String(data.limit ?? "")})`;
      case "file.written":
        return `${ui("changed a file")} ${typeof data.path === "string" ? data.path : ""}`.trim();
      case "file.moved":
        return `${ui("moved a file")} ${typeof data.path === "string" ? data.path : ""}`.trim();
      case "file.deleted":
        return `${ui("deleted a file")} ${typeof data.path === "string" ? data.path : ""}`.trim();
      default:
        return event.type;
    }
  };
  const ordered = [...events].reverse();
  if (ordered.length === 0) {
    return (
      <p className="px-4 py-10 text-center text-sm text-muted-foreground">
        {ui("Nothing has happened yet.")}
      </p>
    );
  }
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead className="w-44">{ui("Time")}</TableHead>
          <TableHead className="w-48">{ui("By")}</TableHead>
          <TableHead>{ui("What happened")}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {ordered.map((event) => (
          <TableRow key={event.seq}>
            <TableCell className="text-xs text-muted-foreground tabular-nums">
              {formatDate(event.created_at, displayLocale)}
            </TableCell>
            <TableCell className="text-[13px]">{name(event.member_id)}</TableCell>
            <TableCell className="text-[13px]">
              {describe(event)}
              <span className="ml-2 font-mono text-[11px] text-muted-foreground">{event.type}</span>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

export function RoomMembersTable({
  members,
  events,
  playground,
}: {
  members: RoomMember[];
  events: RoomEvent[];
  playground?: boolean;
}) {
  const ui = useUiTranslations();
  const kindLabel = useMemberKindLabel();
  const wakeLabel = useWakeModeLabel();
  const sessions = sessionsByMember(events);
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>{ui("Member")}</TableHead>
          <TableHead>{ui("Kind")}</TableHead>
          <TableHead>{ui("Wakes")}</TableHead>
          <TableHead>{ui("Sessions")}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {members.map((member) => {
          const memberSessions = sessions.get(member.id) ?? [];
          return (
            <TableRow key={member.id} className={member.left ? "opacity-60" : undefined}>
              <TableCell>
                <span className="flex items-center gap-2.5">
                  <MemberTile
                    name={member.display_name}
                    kind={member.kind}
                    className="size-6 text-[11px]"
                  />
                  <span className="min-w-0">
                    <span className="block truncate text-[13px] font-medium">
                      {member.display_name}
                    </span>
                    <span className="block font-mono text-[11px] text-muted-foreground">
                      @{member.handle}
                      {member.role === "owner" ? ` · ${ui("owner")}` : ""}
                      {member.left ? ` · ${ui("left")}` : ""}
                    </span>
                  </span>
                </span>
              </TableCell>
              <TableCell className="text-[13px] text-muted-foreground">
                {kindLabel(member.kind, playground)}
              </TableCell>
              <TableCell className="text-[13px]">
                {member.kind === "agent"
                  ? wakeLabel(member.wake)
                  : member.kind === "outside_agent"
                    ? ui("Its client decides")
                    : "—"}
              </TableCell>
              <TableCell className="text-[13px]">
                {member.kind !== "agent" ? (
                  "—"
                ) : memberSessions.length === 0 ? (
                  <span className="text-muted-foreground">
                    {member.session_scope === "room"
                      ? ui("One for the room")
                      : ui("One per thread")}{" "}
                    · 0
                  </span>
                ) : (
                  <span className="flex flex-wrap items-center gap-x-2 text-muted-foreground">
                    {member.session_scope === "room"
                      ? ui("One for the room")
                      : ui("One per thread")}
                    :
                    {memberSessions.map((sessionId, index) => (
                      <Link
                        key={sessionId}
                        href={`/sessions/${sessionId}`}
                        className="text-primary hover:underline"
                      >
                        {index + 1}
                      </Link>
                    ))}
                  </span>
                )}
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export function RoomApiSnippet({ room, lastSeq }: { room: Room; lastSeq?: number }) {
  const ui = useUiTranslations();
  const after = lastSeq ?? 0;
  const http = [
    `# ${ui("Follow the room's changes from where you stopped")}`,
    `GET /v1/rooms/${room.id}/events?after=${after}`,
    "",
    `# ${ui("Post for one of your app's people, the way an app does")}`,
    `POST /v1/rooms/${room.id}/messages`,
    `{"text": "@agent can you look at this?", "as_member": "rmem_..."}`,
  ].join("\n");
  const cli = [
    `everruns rooms events --room-id ${room.id} --after ${after}`,
    `everruns rooms messages list --room-id ${room.id}`,
    `everruns rooms files list --room-id ${room.id}`,
  ].join("\n");
  return (
    <CodeBlock
      samples={[
        { label: "HTTP", language: "http", code: http },
        { label: "CLI", language: "bash", code: cli },
      ]}
    />
  );
}
