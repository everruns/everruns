"use client";

// One run in the operational Sessions list (EVE-853).
//
// The row answers "what ran, on whose behalf, and how did it go" in a single
// scan: a status dot, source-and-subject as the primary line, the agent
// underneath, then the outcome column, duration and when. The outcome column is
// the one that earns its place — on a failed run it replaces the work summary,
// because on a failing run nothing else on the row matters.

import Link from "next/link";
import { cn, shortenId } from "@/lib/utils";
import { formatDurationCompact, formatRelativeTime, formatTokens } from "@/lib/formatting";
import { activityLabel, sourceLabel } from "@/lib/session-filters";
import type { Session, SessionActivity } from "@/lib/api/types";

/** Dot colour per derived activity. Running also pulses so live runs stand out. */
const ACTIVITY_DOT: Record<SessionActivity, string> = {
  running: "bg-primary animate-pulse",
  failed: "bg-destructive",
  completed: "bg-emerald-500",
  paused: "bg-amber-500",
  idle: "bg-muted-foreground/40",
};

/**
 * Wall-clock duration of the run, or undefined when the session has not started
 * or is still going (an in-flight run has no duration yet, and inventing one
 * from `now` would make the column tick while nothing changed).
 */
function runDuration(session: Session): string | undefined {
  if (!session.started_at || !session.finished_at) return undefined;
  const ms = new Date(session.finished_at).getTime() - new Date(session.started_at).getTime();
  if (!Number.isFinite(ms) || ms < 0) return undefined;
  return formatDurationCompact(ms);
}

/** Total tokens across the run, when any were consumed. */
function totalTokens(session: Session): number | undefined {
  const usage = session.usage;
  if (!usage) return undefined;
  const total = usage.input_tokens + usage.output_tokens;
  return total > 0 ? total : undefined;
}

export interface SessionRowProps {
  session: Session;
  /** Resolved agent display name, when the session names an agent. */
  agentName?: string;
}

export function SessionRow({ session, agentName }: SessionRowProps) {
  const activity = session.activity ?? "idle";
  const failed = activity === "failed";
  const subject = session.title?.trim() || session.preview?.trim() || shortenId(session.id);
  const duration = runDuration(session);
  const tokens = totalTokens(session);

  return (
    <Link
      href={`/sessions/${session.id}/transcript`}
      className="flex items-start gap-3 border bg-card px-4 py-3 transition-colors hover:border-foreground/20 hover:bg-accent/10"
    >
      <span
        aria-hidden
        className={cn("mt-1.5 size-2 flex-none rounded-full", ACTIVITY_DOT[activity])}
      />

      <div className="flex min-w-0 flex-[1_1_18rem] flex-col gap-0.5">
        <div className="flex min-w-0 items-baseline gap-1.5 text-sm">
          <span className="flex-none font-medium text-foreground">
            {sourceLabel(session.source ?? "unknown")}
          </span>
          <span aria-hidden className="flex-none text-muted-foreground/50">
            ·
          </span>
          <span className="truncate text-foreground/80">{subject}</span>
        </div>
        <span className="truncate text-[13px] text-muted-foreground">
          {agentName ?? "No agent"}
        </span>
      </div>

      {/*
        Outcome: the failure replaces the work summary, never sits beside it.
        This is the column that earns its place, so it survives down to a narrow
        main column — the rail alone keeps the page under the wider breakpoints.
      */}
      <div className="hidden min-w-0 flex-[1_1_10rem] text-[13px] @lg/page-main:block">
        {failed ? (
          <span
            className="truncate font-medium text-destructive"
            title={session.output_preview ?? undefined}
          >
            {session.output_preview?.trim() || activityLabel(activity)}
          </span>
        ) : (
          <span className="truncate text-muted-foreground">
            {tokens !== undefined ? `${formatTokens(tokens)} tokens` : "No work recorded"}
          </span>
        )}
      </div>

      <span className="hidden w-16 flex-none text-right font-mono text-[13px] text-muted-foreground @md/page-main:block">
        {duration ?? "—"}
      </span>
      <span className="w-24 flex-none text-right text-[13px] text-muted-foreground">
        {formatRelativeTime(session.created_at)}
      </span>
    </Link>
  );
}
