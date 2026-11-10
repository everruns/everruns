"use client";

// The Playground room composer: post as yourself or as a test user, at the
// top level or in a thread, and see before sending who the message wakes.

import { useState } from "react";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { usePostRoomMessage } from "@/hooks/use-rooms";
import type { Room, RoomMember, RoomMessage } from "@/lib/api/rooms";
import { draftWakes } from "@/lib/rooms";
import { useUiTranslations } from "@/providers/locale-provider";

export function RoomComposer({
  room,
  members,
  you,
  canPostForOthers,
  replyTo,
  onCancelReply,
}: {
  room: Room;
  members: RoomMember[];
  /** Your own member, once you have one; posting makes you one. */
  you?: RoomMember;
  canPostForOthers: boolean;
  replyTo?: RoomMessage;
  onCancelReply: () => void;
}) {
  const ui = useUiTranslations();
  const post = usePostRoomMessage(room.id);
  const [text, setText] = useState("");
  const testUsers = members.filter((member) => member.kind === "guest" && !member.left);
  const [asMember, setAsMember] = useState<string>(
    canPostForOthers && testUsers[0] ? testUsers[0].id : "you",
  );
  const wakes = draftWakes(text, members, !!replyTo);
  const agents = members.filter((member) => member.kind === "agent" && !member.left);

  const send = async () => {
    const body = text.trim();
    if (!body) return;
    await post.mutateAsync({
      text: body,
      reply_to: replyTo?.id,
      as_member: asMember === "you" ? undefined : asMember,
    });
    setText("");
    onCancelReply();
  };

  const hint = (() => {
    const names = [
      ...wakes.mentioned,
      ...wakes.everything.filter((a) => !wakes.mentioned.includes(a)),
    ];
    if (names.length)
      return `${ui("Wakes")}: ${names.map((agent) => agent.display_name).join(", ")}`;
    if (wakes.thread) return ui("Wakes the agents already in this thread");
    return ui("Nobody mentioned: nobody wakes. Type @ and an agent's handle.");
  })();

  if (room.archived) {
    return (
      <p className="border bg-card px-4 py-3 text-sm text-muted-foreground">
        {ui("This room is archived: it takes no posts and wakes no agents.")}
      </p>
    );
  }

  return (
    <form
      className="border bg-card"
      onSubmit={(event) => {
        event.preventDefault();
        void send();
      }}
    >
      {replyTo && (
        <div className="flex items-center gap-2 border-b px-3 py-1.5 text-xs text-muted-foreground">
          <span className="min-w-0 flex-1 truncate">
            {ui("Replying in the thread of")} {replyTo.author?.display_name}: {replyTo.text}
          </span>
          <button type="button" aria-label={ui("Cancel reply")} onClick={onCancelReply}>
            <X className="size-3.5" />
          </button>
        </div>
      )}
      <Textarea
        aria-label={ui("Message")}
        className="min-h-20 resize-y border-0 shadow-none focus-visible:ring-0"
        placeholder={
          agents.length
            ? `${ui("Write a message. Mention an agent to wake it, such as")} @${agents[0].handle}`
            : ui("Write a message")
        }
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void send();
          }
        }}
      />
      <div className="flex flex-wrap items-center gap-2 border-t px-3 py-2">
        <span className="text-xs text-muted-foreground">{ui("Post as")}</span>
        <Select value={asMember} onValueChange={setAsMember}>
          <SelectTrigger size="sm" aria-label={ui("Post as")} className="w-auto min-w-36">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="you">
              {you ? `${you.display_name} (${ui("you")})` : ui("You")}
            </SelectItem>
            {canPostForOthers &&
              testUsers.map((member) => (
                <SelectItem key={member.id} value={member.id}>
                  {member.display_name}
                </SelectItem>
              ))}
          </SelectContent>
        </Select>
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{hint}</span>
        {post.error && (
          <span role="alert" className="text-xs text-destructive">
            {post.error.message}
          </span>
        )}
        <Button type="submit" size="sm" loading={post.isPending} disabled={!text.trim()}>
          {replyTo ? ui("Reply") : ui("Post")}
        </Button>
      </div>
    </form>
  );
}
