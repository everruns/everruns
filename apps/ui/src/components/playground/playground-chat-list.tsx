"use client";

import { AgentIcon } from "@/components/icons/facet-icons";
import type { ReactNode } from "react";
import Link from "next/link";
import {
  CircleAlert,
  CircleCheck,
  CircleDashed,
  CirclePause,
  Clock,
  Loader2,
  MessageSquare,
  UserRound,
} from "lucide-react";
import type { Session } from "@/lib/api/types";
import { threadTitle } from "@/lib/chat-threads";
import { getDisplayName } from "@/lib/entity-lifecycle";
import {
  groupPlaygroundSessions,
  playgroundPreview,
  playgroundRowTime,
  playgroundStarter,
  playgroundStatus,
  playgroundStatusLabel,
  type PlaygroundGroupBy,
  type PlaygroundRowStatus,
} from "@/lib/playground-list";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { cn } from "@/lib/utils";

const ROW_GRID = "grid grid-cols-[18px_minmax(0,1fr)_auto_44px_22px_64px] items-center gap-3";

function StatusGlyph({ status }: { status: PlaygroundRowStatus }) {
  const label = playgroundStatusLabel(status);
  const className = "size-3.5";
  let icon = <CircleDashed className={cn(className, "text-muted-foreground")} />;
  if (status === "running") {
    icon = <Loader2 className={cn(className, "animate-spin text-accent-foreground")} />;
  } else if (status === "completed") {
    icon = <CircleCheck className={cn(className, "text-muted-foreground")} />;
  } else if (status === "failed") {
    icon = <CircleAlert className={cn(className, "text-destructive")} />;
  } else if (status === "paused") {
    icon = <CirclePause className={cn(className, "text-accent-foreground")} />;
  } else if (status === "idle") {
    icon = <Clock className={cn(className, "text-accent-foreground")} />;
  }
  return (
    <span className="flex" title={label} aria-label={label}>
      {icon}
    </span>
  );
}

function FactChip({
  href,
  icon,
  children,
}: {
  href?: string;
  icon: ReactNode;
  children: ReactNode;
}) {
  const className =
    "inline-flex h-[22px] max-w-48 items-center gap-1 border px-1.5 text-xs whitespace-nowrap";
  const body = (
    <>
      {icon}
      <span className="truncate">{children}</span>
    </>
  );
  if (!href) {
    return <span className={className}>{body}</span>;
  }
  return (
    <Link href={href} className={cn(className, "relative z-20 hover:bg-muted")}>
      {body}
    </Link>
  );
}

function SubjectChip({ id }: { id?: string | null }) {
  const { data } = useVirtualUser(id ?? undefined);
  if (!id) {
    return <FactChip icon={<UserRound className="size-3 opacity-60" />}>—</FactChip>;
  }
  return (
    <FactChip href={`/virtual-users/${id}`} icon={<UserRound className="size-3 opacity-60" />}>
      {data?.name ?? id}
    </FactChip>
  );
}

function agentLabel(
  session: Session,
  agents: Array<{ id: string; name: string; display_name?: string | null }>,
): string {
  if (!session.agent_id) return "Harness chat";
  const agent = agents.find((candidate) => candidate.id === session.agent_id);
  return agent ? getDisplayName(agent) : session.agent_id;
}

export function PlaygroundChatList({
  sessions,
  agents,
  groupBy,
  now = new Date(),
}: {
  sessions: Session[];
  agents: Array<{ id: string; name: string; display_name?: string | null }>;
  groupBy: PlaygroundGroupBy;
  now?: Date;
}) {
  const groups = groupPlaygroundSessions(
    sessions.map((session) => ({
      session,
      updatedAt: session.updated_at,
      agentLabel: agentLabel(session, agents),
    })),
    groupBy,
    now,
  );

  return (
    <div className="overflow-x-auto border bg-card">
      <div className="min-w-[720px]">
        {groups.map((group) => (
          <div
            key={group.label || "all"}
            role={groupBy === "none" ? undefined : "group"}
            aria-label={groupBy === "none" ? undefined : group.label}
          >
            {groupBy !== "none" && (
              <div className="flex items-center gap-2 border-b bg-muted px-4 py-1.5 text-xs">
                <span className="font-semibold">{group.label}</span>
                <span className="font-mono text-muted-foreground">{group.rows.length}</span>
              </div>
            )}
            {group.rows.map(({ session, agentLabel: label }) => {
              const title = threadTitle(session, "New Playground chat");
              const starter = playgroundStarter(session);
              const eventCount = session.event_count ?? 0;
              return (
                <div
                  key={session.id}
                  className={cn(ROW_GRID, "relative h-10 border-b px-4 text-[13px] hover:bg-muted")}
                >
                  <Link
                    href={`/playground/${session.id}`}
                    aria-label={title}
                    className="absolute inset-0 z-10 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
                  />
                  <StatusGlyph status={playgroundStatus(session)} />
                  <div className="flex min-w-0 items-baseline gap-2.5 overflow-hidden whitespace-nowrap">
                    <span className="max-w-[60%] shrink-0 truncate font-medium">{title}</span>
                    <span className="truncate text-xs text-muted-foreground">
                      {playgroundPreview(session)}
                    </span>
                  </div>
                  <div className="flex justify-end gap-1.5">
                    <FactChip
                      href={session.agent_id ? `/agents/${session.agent_id}` : undefined}
                      icon={<AgentIcon className="size-3 opacity-60" />}
                    >
                      {label}
                    </FactChip>
                    <SubjectChip id={session.playground_user_id} />
                  </div>
                  <span
                    title={`${eventCount} events`}
                    className="inline-flex items-center justify-end gap-1 font-mono text-[11px] text-muted-foreground"
                  >
                    <MessageSquare className="size-2.5 opacity-60" />
                    {eventCount}
                  </span>
                  {starter ? (
                    <span
                      title={`Started by ${starter.name}`}
                      className="inline-flex size-5 items-center justify-center bg-primary text-[9px] font-semibold text-primary-foreground"
                    >
                      {starter.initials}
                    </span>
                  ) : (
                    <span />
                  )}
                  <span className="text-right text-xs whitespace-nowrap text-muted-foreground">
                    {playgroundRowTime(session.updated_at, now)}
                  </span>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
