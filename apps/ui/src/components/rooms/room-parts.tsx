"use client";

// Small pieces every room view shares: member tiles, kind labels, wake chips
// and the source badge. Room content (names, messages, files) is authored data
// and stays outside translation calls.

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { useUiTranslations } from "@/providers/locale-provider";
import type { Room, RoomMember, RoomMemberKind, WakeMode } from "@/lib/api/rooms";
import { roomSource, type MessageListen, type MessageWake } from "@/lib/rooms";
import { cn } from "@/lib/utils";

const KIND_TILE: Record<RoomMemberKind, string> = {
  agent: "bg-accent/15 text-accent-foreground",
  person: "bg-foreground text-background",
  guest: "bg-foreground text-background",
  outside_agent: "bg-warning/15 text-warning",
};

/** A square initial tile for a member, by kind. */
export function MemberTile({
  name,
  kind,
  className,
}: {
  name: string;
  kind: RoomMemberKind;
  className?: string;
}) {
  const initial = name.trim().charAt(0).toUpperCase() || "?";
  return (
    <span
      aria-hidden
      className={cn(
        "inline-flex size-8 shrink-0 items-center justify-center text-[13px] font-semibold",
        KIND_TILE[kind],
        kind === "person" || kind === "guest" ? "rounded-full" : "",
        className,
      )}
    >
      {initial}
    </span>
  );
}

export function useMemberKindLabel() {
  const ui = useUiTranslations();
  return (kind: RoomMemberKind, playground = false): string => {
    switch (kind) {
      case "agent":
        return ui("Everruns agent");
      case "person":
        return ui("Person");
      case "outside_agent":
        return ui("Outside agent");
      case "guest":
        return playground ? ui("Test user") : ui("Guest");
    }
  };
}

export function useWakeModeLabel() {
  const ui = useUiTranslations();
  return (wake: WakeMode): string => {
    switch (wake) {
      case "mentions":
        return ui("Mentions and threads");
      case "everything":
        return ui("Every message");
      case "relevant":
        return ui("Relevant messages");
      case "never":
        return ui("Never wakes");
    }
  };
}

export function RoomSourceBadge({ room }: { room: Pick<Room, "metadata" | "external_ref"> }) {
  const ui = useUiTranslations();
  return roomSource(room) === "playground" ? (
    <Badge variant="outline">{ui("Playground")}</Badge>
  ) : (
    <Badge variant="accent">{ui("API")}</Badge>
  );
}

/** A check's confidence as the chips show it, such as "(94%)". */
function percent(probability?: number): string {
  return probability === undefined ? "" : `(${Math.round(probability * 100)}%)`;
}

/** One chip per agent a relevance check left quiet, with the check's confidence. */
export function ListenChips({
  listens,
  members,
}: {
  listens: MessageListen[];
  members: Map<string, RoomMember>;
}) {
  const ui = useUiTranslations();
  if (listens.length === 0) return null;
  return (
    <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11px]">
      <span className="text-muted-foreground">{ui("Listened")}</span>
      {listens.map((listen) => {
        const name = members.get(listen.memberId)?.display_name ?? listen.handle;
        const why = listen.unavailable
          ? ui("check unavailable")
          : [ui("stayed quiet"), percent(listen.probability)].filter(Boolean).join(" ");
        return (
          <span
            key={listen.memberId}
            className="inline-flex items-center border border-border px-1.5 py-0.5 text-muted-foreground"
          >
            {`${name} · ${why}`}
          </span>
        );
      })}
    </div>
  );
}

/** One chip per agent a message woke, with why, or the limit that stopped it. */
export function WakeChips({
  wakes,
  members,
  sessionHref,
}: {
  wakes: MessageWake[];
  members: Map<string, RoomMember>;
  sessionHref?: (sessionId: string) => string;
}) {
  const ui = useUiTranslations();
  if (wakes.length === 0) return null;
  const reasonLabel = (wake: MessageWake) => {
    switch (wake.reason) {
      case "mentioned":
        return ui("mentioned");
      case "thread":
        return ui("reply in its thread");
      case "everything":
        return ui("wakes on every message");
      case "relevant":
        return [ui("relevant to it"), percent(wake.probability)].filter(Boolean).join(" ");
      case "limited":
        return `${ui("stopped by a limit")}: ${wake.limit ?? ""}`.trim();
      case "failed":
        return ui("failed");
    }
  };
  return (
    <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11px]">
      <span className="text-muted-foreground">{ui("Woke")}</span>
      {wakes.map((wake) => {
        const name = members.get(wake.memberId)?.display_name ?? wake.handle;
        const label = `${name} · ${reasonLabel(wake)}`;
        const chip = (
          <span
            className={cn(
              "inline-flex items-center border px-1.5 py-0.5",
              wake.reason === "limited"
                ? "border-warning/40 bg-warning/10 text-warning"
                : "border-accent/40 bg-accent/10 text-foreground",
            )}
          >
            {label}
          </span>
        );
        return wake.sessionId && sessionHref ? (
          <Link
            key={`${wake.memberId}-${wake.sessionId}`}
            href={sessionHref(wake.sessionId)}
            className="hover:underline"
            title={ui("Open the agent's session")}
          >
            {chip}
          </Link>
        ) : (
          <span key={`${wake.memberId}-${wake.reason}`}>{chip}</span>
        );
      })}
    </div>
  );
}
