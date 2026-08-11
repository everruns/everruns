/**
 * Inline run card: the work a chat turn kicked off, shown in the transcript
 * where the turn ended so the run is one click away instead of one navigation
 * away. Placement is derived in `run-cards.ts`.
 */
"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Bot, Cpu, ExternalLink, ListTodo, Radar } from "lucide-react";
import type { SessionTask, SessionTaskState } from "@/lib/api/types";
import { formatWorkedDuration } from "@/components/chat/turn-delimiter";
import { runDurationMs, type ChatRun } from "@/components/chat/run-cards";
import { cn } from "@/lib/utils";

const STATUS_DOT: Record<SessionTaskState, string> = {
  queued: "bg-muted-foreground/60",
  running: "bg-primary animate-pulse",
  awaiting_input: "bg-amber-500",
  succeeded: "bg-emerald-500",
  failed: "bg-destructive",
  canceled: "bg-muted-foreground/40",
};

const STATUS_LABEL: Record<SessionTaskState, string> = {
  queued: "Queued",
  running: "Running",
  awaiting_input: "Needs input",
  succeeded: "Succeeded",
  failed: "Failed",
  canceled: "Canceled",
};

function kindIcon(kind: string) {
  if (kind === "subagent" || kind === "external_agent") return <Bot className="h-3.5 w-3.5" />;
  if (kind === "background_tool") return <Cpu className="h-3.5 w-3.5" />;
  if (kind === "monitor") return <Radar className="h-3.5 w-3.5" />;
  return <ListTodo className="h-3.5 w-3.5" />;
}

/** Where "Open session" goes: the run's own transcript when it has one,
 *  otherwise the owning session's task list. */
function runHref(task: SessionTask): string {
  const childSessionId = task.links?.child_session_id;
  return childSessionId
    ? `/sessions/${childSessionId}/transcript`
    : `/sessions/${task.session_id}/resources`;
}

export function RunCard({ run, now }: { run: ChatRun; now: number }) {
  const { task, subagentCount } = run;
  const durationMs = runDurationMs(task, now);
  const label = task.display_name || task.id;

  return (
    <div
      data-slot="run-card"
      className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5 border border-border/70 bg-card/90 px-3 py-2 text-[13px] shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]"
    >
      <span className="flex min-w-0 items-center gap-2">
        <span aria-hidden="true" className={cn("h-2 w-2 flex-none", STATUS_DOT[task.state])} />
        <span className="sr-only">{STATUS_LABEL[task.state]}: </span>
        <span className="flex-none text-muted-foreground">{kindIcon(task.kind)}</span>
        <span className="truncate font-medium text-foreground">{label}</span>
      </span>

      <span className="flex items-center gap-3 text-xs text-muted-foreground">
        <span>{STATUS_LABEL[task.state]}</span>
        {subagentCount > 0 && (
          <span>
            {subagentCount} {subagentCount === 1 ? "subagent" : "subagents"}
          </span>
        )}
        {durationMs != null && <span>{formatWorkedDuration(durationMs)}</span>}
      </span>

      <Link
        href={runHref(task)}
        className="ml-auto inline-flex items-center gap-1.5 border border-border/70 px-2 py-1 text-xs font-medium text-foreground transition-colors hover:bg-muted/40"
      >
        <ExternalLink className="h-3 w-3" />
        Open session
      </Link>
    </div>
  );
}

/** Wall clock that ticks only while something is still running. */
function useRunClock(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active]);

  return now;
}

export function RunCards({ runs }: { runs: ChatRun[] }) {
  const now = useRunClock(runs.some((run) => run.task.started_at && !run.task.finished_at));

  if (runs.length === 0) return null;
  return (
    <div role="list" aria-label="Runs started by this turn">
      {runs.map((run) => (
        <div role="listitem" key={run.task.id}>
          <RunCard run={run} now={now} />
        </div>
      ))}
    </div>
  );
}
