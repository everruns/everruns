"use client";

// The whole run a session belongs to: the session Agents tab, the Trace
// "Whole run" timeline and the Cost "Whole run" split. Read-only: the session
// page is for debugging, and a request to the person is answered where the
// person talks (knowledge/runtime-resources/agents-using-agents.md).
//
// Every number comes from GET /v1/sessions/{id}/run.

import Link from "next/link";
import { useMemo } from "react";
import type { SessionRun } from "@/lib/api/schema-types";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";
import { CostCard, formatSeconds, formatUsd } from "@/components/session/cost/cost-shared";
import { SegmentedControl } from "@/components/ui/segmented-control";

type RunSession = SessionRun["sessions"][number];

/** Lane colors, cycled by position in the run (the Cost tab's palette). */
const LANE_COLORS = [
  "bg-[#8a5cc4] dark:bg-[#a07ee0]",
  "bg-[#6b7280] dark:bg-[#9ca3af]",
  "bg-[#b07a12] dark:bg-[#b98a22]",
  "bg-[#2c4f8f] dark:bg-[#5b86d6]",
  "bg-[#b5487a] dark:bg-[#d06a98]",
  "bg-[#0a9396] dark:bg-[#17a598]",
] as const;

export function runAgentName(session: RunSession, fallback: string): string {
  return session.agent?.name ?? session.title ?? fallback;
}

function initial(name: string): string {
  return name.trim().charAt(0).toUpperCase() || "?";
}

/** A session still working: its turn runs, or it waits on someone. */
function isWorking(session: RunSession): boolean {
  return session.turns.some((turn) => !turn.ended_at) || session.status === "active";
}

/** Seconds from the session's first turn to its last turn's end (or now). */
function workedSeconds(session: RunSession, now: number): number | null {
  const first = session.turns[0];
  if (!first) return null;
  const last = session.turns[session.turns.length - 1];
  const end = last.ended_at ? Date.parse(last.ended_at) : now;
  return Math.max(0, (end - Date.parse(first.started_at)) / 1000);
}

function StatusBadge({ session }: { session: RunSession }) {
  const ui = useUiTranslations();
  let label: string;
  let tone: string;
  if (session.status === "waiting_for_tool_results") {
    label = ui("Waiting on the person");
    tone = "border-amber-500/40 text-amber-700 dark:text-amber-300";
  } else if (isWorking(session)) {
    label = ui("Working");
    tone = "border-primary/40 text-primary";
  } else if (session.last_turn_status === "failed") {
    label = ui("Failed");
    tone = "border-destructive/40 text-destructive";
  } else if (session.last_turn_status === "cancelled") {
    label = ui("Cancelled");
    tone = "border-border text-muted-foreground";
  } else if (session.turns.length === 0) {
    label = ui("Not started");
    tone = "border-border text-muted-foreground";
  } else {
    label = ui("Done");
    tone = "border-emerald-500/40 text-emerald-700 dark:text-emerald-300";
  }
  return <span className={cn("inline-block border px-2 py-0.5 text-xs", tone)}>{label}</span>;
}

function AgentTile({ name, index }: { name: string; index: number }) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex h-8 w-8 shrink-0 items-center justify-center text-sm font-semibold text-white",
        LANE_COLORS[index % LANE_COLORS.length],
      )}
    >
      {initial(name)}
    </span>
  );
}

/** Run cost, the first session's share, and what waits on the person. */
export function RunSummaryCards({ run }: { run: SessionRun }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const first = run.sessions[0];
  const firstName = first ? runAgentName(first, ui("First session")) : ui("First session");
  const others = run.sessions.length - 1;
  return (
    <div className="grid border border-border bg-card sm:grid-cols-3">
      <div className="border-b border-border p-4 sm:border-r sm:border-b-0">
        <p className="text-xs text-muted-foreground">{ui("Run cost so far")}</p>
        <p className="mt-1 text-3xl font-semibold tabular-nums">
          {formatUsd(run.cost_usd, locale)}
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          {ui("{value0} {value1} · other agents {value2}", {
            value0: firstName,
            value1: formatUsd(run.first_session_cost_usd, locale),
            value2: formatUsd(run.cost_usd - run.first_session_cost_usd, locale),
          })}
        </p>
      </div>
      <div className="border-b border-border p-4 sm:border-r sm:border-b-0">
        <p className="text-xs text-muted-foreground">{ui("Agents in this run")}</p>
        <p className="mt-1 text-3xl font-semibold tabular-nums">{run.sessions.length}</p>
        <p className="mt-1 text-xs text-muted-foreground">
          {others === 1
            ? ui("1 hand-off")
            : ui("{value0} hand-offs", { value0: others.toLocaleString(locale) })}
        </p>
      </div>
      <div className="p-4">
        <p className="text-xs text-muted-foreground">{ui("Waiting on the person")}</p>
        <p className="mt-1 text-lg font-semibold">
          {run.waiting_requests === 0
            ? ui("Nothing")
            : run.waiting_requests === 1
              ? ui("1 request")
              : ui("{value0} requests", { value0: run.waiting_requests })}
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          {ui("Answered in the conversation the person is in")}
        </p>
      </div>
    </div>
  );
}

/** Every hand-off in the run, with status, time and cost. */
export function RunHandoffs({ run, sessionId }: { run: SessionRun; sessionId: string }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const now = Date.now();
  const handoffs = run.sessions.slice(1);
  return (
    <CostCard title={ui("Work handed off")} meta={ui("Open a row for that agent's session")}>
      {handoffs.length === 0 ? (
        <p className="p-4 text-sm text-muted-foreground">
          {ui("No work has been handed to other agents in this run yet.")}
        </p>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-xs text-muted-foreground">
              <th className="w-12 px-4 py-2" />
              <th className="px-4 py-2 text-left font-medium">{ui("Agent and task")}</th>
              <th className="px-4 py-2 text-left font-medium">{ui("Status")}</th>
              <th className="px-4 py-2 text-right font-medium">{ui("Time")}</th>
              <th className="px-4 py-2 text-right font-medium">{ui("Cost")}</th>
            </tr>
          </thead>
          <tbody>
            {handoffs.map((session, index) => {
              const name = runAgentName(session, ui("Agent"));
              const seconds = workedSeconds(session, now);
              const current = session.session_id === sessionId;
              return (
                <tr
                  key={session.session_id}
                  className={cn(
                    "border-b border-border/60 last:border-b-0",
                    current && "bg-muted/40",
                  )}
                >
                  <td className="px-4 py-2.5">
                    <AgentTile name={name} index={index + 1} />
                  </td>
                  <td className="max-w-0 px-4 py-2.5">
                    <Link
                      href={`/sessions/${session.session_id}/trace`}
                      className="block truncate font-medium hover:underline"
                    >
                      {name}
                    </Link>
                    {session.title && session.title !== name ? (
                      <p className="truncate text-xs text-muted-foreground">{session.title}</p>
                    ) : null}
                  </td>
                  <td className="px-4 py-2.5">
                    <StatusBadge session={session} />
                  </td>
                  <td className="px-4 py-2.5 text-right tabular-nums">
                    {seconds == null ? "–" : formatSeconds(seconds)}
                  </td>
                  <td className="px-4 py-2.5 text-right tabular-nums">
                    {session.turns.length === 0 && session.cost_usd === 0
                      ? "–"
                      : formatUsd(session.cost_usd, locale)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {run.truncated ? (
        <p className="border-t border-border px-4 py-2.5 text-xs text-muted-foreground">
          {ui("This run is large; the oldest turns are not shown.")}
        </p>
      ) : null}
    </CostCard>
  );
}

/**
 * One lane per agent session; bars are turns. A bar opens that session's
 * trace. Dashed lines mark where a session was handed its work.
 */
export function RunTimeline({ run, sessionId }: { run: SessionRun; sessionId: string }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const span = useMemo(() => {
    const now = Date.now();
    let start = Number.POSITIVE_INFINITY;
    let end = 0;
    for (const session of run.sessions) {
      for (const turn of session.turns) {
        start = Math.min(start, Date.parse(turn.started_at));
        end = Math.max(end, turn.ended_at ? Date.parse(turn.ended_at) : now);
      }
    }
    if (!Number.isFinite(start)) return null;
    return { start, end, length: Math.max(1, end - start), now };
  }, [run]);

  if (!span) {
    return (
      <CostCard title={ui("Run timeline")}>
        <p className="p-4 text-sm text-muted-foreground">{ui("No turns have run yet.")}</p>
      </CostCard>
    );
  }
  const at = (iso: string) => ((Date.parse(iso) - span.start) / span.length) * 100;
  const ticks = [0, 0.25, 0.5, 0.75, 1];
  return (
    <CostCard
      title={ui("Run timeline")}
      meta={ui("Bars are turns; dashed lines are hand-offs. Open a bar for that turn's agent.")}
    >
      <div className="overflow-x-auto">
        <div className="min-w-[640px]">
          {run.sessions.map((session, index) => {
            const name = runAgentName(session, ui("Agent"));
            const handedAt = index > 0 ? at(session.created_at) : null;
            return (
              <div
                key={session.session_id}
                className={cn(
                  "grid grid-cols-[14rem_1fr] items-center border-b border-border/60",
                  session.session_id === sessionId && "bg-muted/40",
                )}
              >
                <div className="flex min-w-0 items-center gap-2 px-4 py-2">
                  <AgentTile name={name} index={index} />
                  <span className="truncate text-sm">{name}</span>
                </div>
                <div className="relative h-11">
                  {handedAt != null && handedAt >= 0 && handedAt <= 100 ? (
                    <span
                      aria-hidden
                      className="absolute top-0 bottom-0 border-l border-dashed border-primary/50"
                      style={{ left: `${handedAt}%` }}
                    />
                  ) : null}
                  {session.turns.map((turn) => {
                    const left = at(turn.started_at);
                    const end = turn.ended_at
                      ? at(turn.ended_at)
                      : ((span.now - span.start) / span.length) * 100;
                    return (
                      <Link
                        key={turn.turn_id}
                        href={`/sessions/${session.session_id}/trace`}
                        title={ui("{value0}, turn {value1} · {value2}", {
                          value0: name,
                          value1: turn.turn_no,
                          value2: formatUsd(turn.cost_usd, locale),
                        })}
                        className={cn(
                          "absolute top-3 h-5 hover:opacity-80",
                          LANE_COLORS[index % LANE_COLORS.length],
                          turn.status === "failed" && "opacity-60",
                        )}
                        style={{
                          left: `${left}%`,
                          width: `max(4px, ${Math.max(0, end - left)}%)`,
                        }}
                      />
                    );
                  })}
                </div>
              </div>
            );
          })}
          <div className="grid grid-cols-[14rem_1fr]">
            <span />
            <div className="relative h-7 text-xs text-muted-foreground">
              {ticks.map((tick) => (
                <span
                  key={tick}
                  className="absolute top-1.5 -translate-x-1/2 tabular-nums first:translate-x-0 last:-translate-x-full"
                  style={{ left: `${tick * 100}%` }}
                >
                  {formatSeconds((span.length / 1000) * tick)}
                </span>
              ))}
            </div>
          </div>
        </div>
      </div>
    </CostCard>
  );
}

/** Whole-run spend split by agent session, largest first. */
export function RunCostSplit({ run }: { run: SessionRun }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const rows = run.sessions
    .map((session, index) => ({ session, index }))
    .sort((a, b) => b.session.cost_usd - a.session.cost_usd);
  return (
    <CostCard
      title={ui("Whole run by agent")}
      meta={ui("{value0} across {value1} sessions", {
        value0: formatUsd(run.cost_usd, locale),
        value1: run.sessions.length,
      })}
    >
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-xs text-muted-foreground">
            <th className="px-4 py-2 text-left font-medium">{ui("Agent")}</th>
            <th className="px-4 py-2 text-right font-medium">{ui("Turns")}</th>
            <th className="px-4 py-2 text-right font-medium">{ui("Share")}</th>
            <th className="px-4 py-2 text-right font-medium">{ui("Cost")}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ session, index }) => (
            <tr key={session.session_id} className="border-b border-border/60 last:border-b-0">
              <td className="max-w-0 px-4 py-2">
                <Link
                  href={`/sessions/${session.session_id}/cost`}
                  className="block truncate hover:underline"
                >
                  {runAgentName(session, ui("Agent"))}
                  {index === 0 ? (
                    <span className="ml-2 text-xs text-muted-foreground">
                      {ui("first session")}
                    </span>
                  ) : null}
                </Link>
              </td>
              <td className="px-4 py-2 text-right text-muted-foreground tabular-nums">
                {session.turns.length}
              </td>
              <td className="px-4 py-2 text-right text-muted-foreground tabular-nums">
                {run.cost_usd > 0 ? `${Math.round((session.cost_usd / run.cost_usd) * 100)}%` : "–"}
              </td>
              <td className="px-4 py-2 text-right tabular-nums">
                {formatUsd(session.cost_usd, locale)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </CostCard>
  );
}

/** "This session / Whole run" switch, shown when other agents took part. */
export function RunScopeSwitch({
  scope,
  onChange,
  agents,
}: {
  scope: "session" | "run";
  onChange: (scope: "session" | "run") => void;
  agents: number;
}) {
  const ui = useUiTranslations();
  return (
    <SegmentedControl
      label={ui("What to show")}
      value={scope}
      onValueChange={(value) => onChange(value === "run" ? "run" : "session")}
      options={[
        { value: "session", label: ui("This session") },
        { value: "run", label: ui("Whole run · {value0} agents", { value0: agents }) },
      ]}
    />
  );
}
