"use client";

// A room's messages as the console shows them: each top-level message with
// who woke and why, its thread folded under it. Read-only in the recording;
// the Playground passes `onReply` to answer in a thread.

import Link from "next/link";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { MessageSquareReply, Pin } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { Skeleton } from "@/components/ui/skeleton";
import {
  listRoomReplies,
  type Room,
  type RoomEvent,
  type RoomMember,
  type RoomMessage,
} from "@/lib/api/rooms";
import { formatDate } from "@/lib/formatting";
import { listensByMessage, wakesByMessage } from "@/lib/rooms";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";
import { ListenChips, MemberTile, WakeChips, useMemberKindLabel } from "./room-parts";

export interface RoomFeedProps {
  room: Room;
  messages: RoomMessage[];
  members: RoomMember[];
  events: RoomEvent[];
  playground?: boolean;
  sessionHref?: (sessionId: string) => string;
  onReply?: (message: RoomMessage) => void;
  /** Refetch interval for open threads, while agents may still answer. */
  refetchInterval?: number | false;
}

export function RoomMessageFeed(props: RoomFeedProps) {
  const ui = useUiTranslations();
  if (props.messages.length === 0) {
    return (
      <p className="border bg-card px-4 py-10 text-center text-sm text-muted-foreground">
        {ui("No messages yet.")}
      </p>
    );
  }
  return (
    <ol className="flex flex-col gap-5" aria-label={ui("Messages")}>
      {props.messages.map((message) => (
        <li key={message.id}>
          <RoomMessageItem {...props} message={message} />
          {message.reply_count > 0 && <RoomThread {...props} root={message} />}
        </li>
      ))}
    </ol>
  );
}

function RoomThread({ root, ...props }: RoomFeedProps & { root: RoomMessage }) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(true);
  const { data: replies, isLoading } = useQuery({
    queryKey: ["rooms", props.room.id, "replies", root.id, root.reply_count],
    queryFn: () => listRoomReplies(props.room.id, root.id),
    enabled: open,
    refetchInterval: props.refetchInterval,
  });
  return (
    <div className="mt-2 ml-11 border-l pl-4">
      <button
        type="button"
        className="text-xs font-medium text-muted-foreground hover:text-foreground"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        {open ? ui("Hide thread") : ui("Show thread")} · {root.reply_count}{" "}
        {root.reply_count === 1 ? ui("reply") : ui("replies")}
      </button>
      {open &&
        (isLoading ? (
          <Skeleton className="mt-2 h-16 w-full" />
        ) : (
          <ol className="mt-3 flex flex-col gap-4">
            {(replies ?? []).map((reply) => (
              <li key={reply.id}>
                <RoomMessageItem {...props} message={reply} />
              </li>
            ))}
          </ol>
        ))}
    </div>
  );
}

function RoomMessageItem({
  room,
  message,
  members,
  events,
  playground,
  sessionHref,
  onReply,
}: RoomFeedProps & { message: RoomMessage }) {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const kindLabel = useMemberKindLabel();
  const memberById = new Map(members.map((member) => [member.id, member]));
  const wakes = wakesByMessage(events, [message], members).get(message.id) ?? [];
  const listens = listensByMessage(events, members).get(message.id) ?? [];
  const author = message.author;
  const maxHops = room.loop_limits.max_hops;

  if (!author) {
    // A notice the room itself posted, such as a loop limit.
    return (
      <div className="flex items-start gap-2 pl-11 text-[13px] text-muted-foreground">
        <span aria-hidden>—</span>
        <span>{message.text}</span>
      </div>
    );
  }

  return (
    <article
      className="flex gap-3"
      aria-label={`${author.display_name} ${formatDate(message.created_at, displayLocale)}`}
    >
      <MemberTile name={author.display_name} kind={author.kind} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold">{author.display_name}</span>
          <Badge variant="outline">{kindLabel(author.kind, playground)}</Badge>
          <time className="text-xs text-muted-foreground" dateTime={message.created_at}>
            {formatDate(message.created_at, displayLocale)}
          </time>
          {message.edited_at && (
            <span className="text-xs text-muted-foreground">{ui("edited")}</span>
          )}
          {message.pinned && (
            <Pin className="size-3 text-muted-foreground" aria-label={ui("Pinned")} />
          )}
        </div>
        {message.deleted ? (
          <p className="mt-1 text-sm text-muted-foreground italic">{ui("Message deleted")}</p>
        ) : (
          <MarkdownDisplay content={message.text} className="mt-1 break-words" />
        )}
        {message.reactions.length > 0 && (
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {message.reactions.map((reaction) => (
              <span
                key={reaction.emoji}
                className="inline-flex items-center gap-1 border bg-card px-1.5 py-0.5 text-xs"
                title={reaction.member_ids
                  .map((id) => memberById.get(id)?.display_name ?? id)
                  .join(", ")}
              >
                {reaction.emoji} {reaction.count}
              </span>
            ))}
          </div>
        )}
        <WakeChips wakes={wakes} members={memberById} sessionHref={sessionHref} />
        <ListenChips listens={listens} members={memberById} />
        <div className="mt-1 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
          {message.hops > 0 && (
            <span>
              {ui("Agent replies in a row")}: {message.hops} / {maxHops}
            </span>
          )}
          {message.session_id && sessionHref && (
            <Link className="text-primary hover:underline" href={sessionHref(message.session_id)}>
              {ui("Turn in the agent's session")}
            </Link>
          )}
          {onReply && !message.thread_id && !room.archived && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-1.5 text-[11px]"
              onClick={() => onReply(message)}
            >
              <MessageSquareReply className="size-3" />
              {ui("Reply in thread")}
            </Button>
          )}
        </div>
      </div>
    </article>
  );
}
