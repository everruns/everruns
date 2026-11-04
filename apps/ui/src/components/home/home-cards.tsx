"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { Check, Plus } from "lucide-react";
import { RunBars } from "@/components/agents/home/run-bars";
import { NewAgentLink } from "@/components/agents/new-agent-link";
import { AgentIcon, HealthIcon, SandboxIcon } from "@/components/icons/facet-icons";
import { Button, buttonVariants } from "@/components/ui/button";
import { formatCompactNumber, formatRelativeTime } from "@/lib/formatting";
import type {
  AgentActivity,
  AgentActivityTotals,
  DoctorReport,
  SandboxFleetStats,
  Session,
  TimelineSummary,
} from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";

export function HomeCard({
  title,
  icon,
  badges,
  link,
  children,
  testId,
}: {
  title: string;
  icon?: ReactNode;
  badges?: ReactNode;
  link?: { href: string; label: string };
  children: ReactNode;
  testId?: string;
}) {
  return (
    <section className="border bg-card" aria-label={title} data-testid={testId}>
      <header className="flex items-center gap-2.5 border-b px-4 py-3">
        {icon}
        <h2 className="text-sm font-semibold">{title}</h2>
        {badges}
        {link && (
          <Link href={link.href} className="ml-auto text-xs underline underline-offset-4">
            {link.label}
          </Link>
        )}
      </header>
      {children}
    </section>
  );
}

/* ───────────────────────────── Key numbers ───────────────────────────── */

function Kpi({
  label,
  value,
  unit,
  hint,
  alert,
  children,
}: {
  label: string;
  value: string;
  unit?: string;
  hint?: ReactNode;
  alert?: boolean;
  children?: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-col gap-1 border-border px-4 py-3.5",
        alert && "shadow-[inset_0_2px_0_hsl(var(--destructive))]",
      )}
    >
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-2xl leading-tight font-semibold tracking-tight tabular-nums">
        {value}
        {unit && <span className="ml-1 text-[13px] font-medium text-muted-foreground">{unit}</span>}
      </span>
      {children}
      {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
    </div>
  );
}

function Delta({
  current,
  previous,
  invert,
}: {
  current: number;
  previous?: number;
  invert?: boolean;
}) {
  const ui = useUiTranslations();
  if (!previous) return null;
  const change = (current - previous) / previous;
  const good = invert ? change <= 0 : change >= 0;
  return (
    <>
      <span className={cn("font-semibold", good ? "text-success" : "text-destructive")}>
        {change >= 0 ? "+" : "−"}
        {Math.abs(Math.round(change * 100))}%
      </span>{" "}
      {ui("vs previous 24h")}
    </>
  );
}

export function HomeKpis({
  totals,
  summary,
  previous,
  liveSandboxes,
  sandboxProviders,
}: {
  totals?: AgentActivityTotals;
  summary?: TimelineSummary;
  previous?: TimelineSummary;
  liveSandboxes?: number;
  sandboxProviders?: string;
}) {
  const ui = useUiTranslations();
  const running = totals?.running_sessions ?? 0;
  const limit = totals?.max_active_turns ?? 0;
  const atLimit = limit > 0 && running >= limit;
  const runs = summary?.turns_started ?? totals?.runs ?? 0;
  const failed = summary?.turns_failed ?? totals?.failed ?? 0;
  const failureRate = summary?.failure_rate ?? (runs ? failed / runs : null);
  const previousRate = previous?.failure_rate ?? null;
  const rising = failureRate !== null && previousRate !== null && failureRate > previousRate * 1.5;
  const pct = (value: number) => `${(value * 100).toFixed(value < 0.1 ? 1 : 0)}%`;
  return (
    <div
      className="grid grid-cols-2 divide-border border bg-card @3xl/home:grid-cols-3 @6xl/home:grid-cols-6 [&>*]:border-t [&>*]:border-l"
      data-testid="home-kpis"
    >
      <Kpi
        label={ui("Running now")}
        value={totals ? String(running) : "–"}
        unit={ui("sessions")}
        alert={atLimit}
        hint={limit ? ui("{running} of {limit} turn limit", { running, limit }) : undefined}
      >
        {limit > 0 && (
          <span className="mt-1 block h-1 bg-muted" aria-hidden="true">
            <span
              className={cn("block h-1", atLimit ? "bg-warning" : "bg-primary")}
              style={{ width: `${Math.min(100, (running / limit) * 100)}%` }}
            />
          </span>
        )}
      </Kpi>
      <Kpi
        label={ui("Turns")}
        value={totals || summary ? formatCompactNumber(runs) : "–"}
        hint={
          summary && previous ? (
            <Delta current={summary.turns_started} previous={previous.turns_started} />
          ) : (
            ui("last 24 hours")
          )
        }
      />
      <Kpi
        label={ui("Failed turns")}
        value={failureRate === null ? "–" : pct(failureRate)}
        unit={failed ? String(failed) : undefined}
        alert={rising}
        hint={
          previousRate !== null
            ? ui("was {rate}", { rate: pct(previousRate) })
            : ui("last 24 hours")
        }
      />
      <Kpi
        label={ui("Channel sessions")}
        value={totals ? formatCompactNumber(totals.channel_sessions) : "–"}
        hint={ui("started by channels, 7 days")}
      />
      <Kpi
        label={ui("People reached")}
        value={totals ? formatCompactNumber(totals.people_reached) : "–"}
        hint={ui("through published channels")}
      />
      <Kpi
        label={ui("Live sandboxes")}
        value={liveSandboxes === undefined ? "–" : String(liveSandboxes)}
        hint={sandboxProviders || undefined}
      />
    </div>
  );
}

/* ─────────────────────────────── Agents ─────────────────────────────── */

export function HomeAgents({
  agents,
  names,
  total,
}: {
  agents: AgentActivity[];
  names: Map<string, string>;
  total: number;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  return (
    <HomeCard
      title={ui("Agents")}
      icon={<AgentIcon className="size-4 text-muted-foreground" aria-hidden="true" />}
      link={{ href: "/agents", label: ui("All {count} agents", { count: total }) }}
      testId="home-agents"
    >
      {agents.length === 0 ? (
        <p className="px-4 py-3 text-[13px] text-muted-foreground">
          {ui("No agent ran in the last 24 hours.")}
        </p>
      ) : (
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b text-left text-xs text-muted-foreground">
              <th className="px-4 py-2 font-medium">{ui("Agent")}</th>
              <th className="px-4 py-2 font-medium">{ui("Now")}</th>
              <th className="hidden px-4 py-2 font-medium @2xl/home:table-cell">
                {ui("Last 24h")}
              </th>
              <th className="px-4 py-2 text-right font-medium">{ui("Turns")}</th>
              <th className="px-4 py-2 text-right font-medium">{ui("Failed")}</th>
              <th className="hidden px-4 py-2 text-right font-medium @xl/home:table-cell">
                {ui("Last turn")}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border/60">
            {agents.map((agent) => {
              const name = names.get(agent.agent_id) ?? agent.agent_id;
              const failing = agent.runs > 0 && agent.failed / agent.runs >= 0.2;
              return (
                <tr key={agent.agent_id}>
                  <td className="px-4 py-2.5 font-semibold">
                    <Link href={`/agents/${agent.agent_id}`} className="hover:underline">
                      {name}
                    </Link>
                  </td>
                  <td className="px-4 py-2.5">
                    <Status
                      tone={agent.running_sessions ? "success" : failing ? "destructive" : "muted"}
                    >
                      {agent.running_sessions
                        ? ui("{count} running", { count: agent.running_sessions })
                        : failing
                          ? ui("Failing")
                          : ui("Idle")}
                    </Status>
                  </td>
                  <td className="hidden px-4 py-2.5 @2xl/home:table-cell">
                    <RunBars
                      buckets={agent.hourly}
                      label={ui("{runs} turns in the last 24 hours", { runs: agent.runs })}
                      className="h-5 w-32"
                    />
                  </td>
                  <td className="px-4 py-2.5 text-right tabular-nums">{agent.runs}</td>
                  <td
                    className={cn(
                      "px-4 py-2.5 text-right tabular-nums",
                      failing && "font-semibold text-destructive",
                    )}
                  >
                    {agent.runs ? `${Math.round((agent.failed / agent.runs) * 1000) / 10}%` : "–"}
                  </td>
                  <td className="hidden px-4 py-2.5 text-right text-muted-foreground @xl/home:table-cell">
                    {agent.last_turn_at
                      ? formatRelativeTime(agent.last_turn_at, displayLocale)
                      : "–"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </HomeCard>
  );
}

function Status({
  tone,
  children,
}: {
  tone: "success" | "destructive" | "warning" | "muted";
  children: ReactNode;
}) {
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
      <span
        aria-hidden="true"
        className={cn(
          "size-1.5",
          tone === "success" && "bg-success",
          tone === "destructive" && "bg-destructive",
          tone === "warning" && "bg-warning",
          tone === "muted" && "bg-muted-foreground/60",
        )}
      />
      {children}
    </span>
  );
}

/* ────────────────────────────── Running now ───────────────────────────── */

export function HomeRunning({
  sessions,
  total,
  names,
}: {
  sessions: Session[];
  total: number;
  names: Map<string, string>;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  if (total === 0) return null;
  return (
    <HomeCard
      title={ui("Running now")}
      badges={
        <span className="text-xs text-muted-foreground">
          {ui("{count} sessions", { count: total })}
        </span>
      }
      link={{ href: "/sessions?status=running", label: ui("All sessions") }}
      testId="home-running"
    >
      <ul className="divide-y divide-border/60 text-[13px]">
        {sessions.map((session) => (
          <li key={session.id} className="flex items-center gap-3 px-4 py-2.5">
            <span className="size-1.5 shrink-0 bg-success" aria-hidden="true" />
            <Link
              href={`/sessions/${session.id}`}
              className="min-w-0 flex-1 truncate font-medium hover:underline"
            >
              {session.title || ui("Untitled session")}
            </Link>
            <span className="hidden min-w-0 truncate text-muted-foreground @xl/home:inline">
              {names.get(session.agent_id ?? "") ?? ""}
            </span>
            <span className="w-20 shrink-0 text-right text-muted-foreground tabular-nums">
              {formatRelativeTime(session.last_message_at ?? session.updated_at, displayLocale)}
            </span>
          </li>
        ))}
      </ul>
    </HomeCard>
  );
}

/* ──────────────────────────── Side cards ──────────────────────────── */

function Fact({ label, value, tone }: { label: string; value: ReactNode; tone?: "warning" }) {
  return (
    <>
      <dt className="text-[13px]">{label}</dt>
      <dd
        className={cn(
          "text-right text-[13px] tabular-nums",
          tone === "warning" && "font-semibold text-warning",
        )}
      >
        {value}
      </dd>
    </>
  );
}

export function HomeHealth({
  open,
  doctor,
}: {
  /** Open health issues, snoozed ones excluded. */
  open: number;
  doctor?: DoctorReport;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const problems = doctor?.checks.filter((check) => check.status === "problem").length ?? 0;
  const warnings = doctor?.checks.filter((check) => check.status === "warning").length ?? 0;
  const clear = open === 0 && problems === 0 && warnings === 0;
  return (
    <HomeCard
      title={ui("Health")}
      icon={<HealthIcon className="size-4 text-muted-foreground" aria-hidden="true" />}
      badges={
        clear ? (
          <span className="border border-success/30 bg-success/10 px-1.5 text-[11px] text-success">
            {ui("All checks pass")}
          </span>
        ) : undefined
      }
      link={{ href: "/settings/health", label: ui("Open") }}
      testId="home-health"
    >
      <dl className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-2 px-4 py-3">
        <Fact
          label={ui("Organization issues")}
          value={open ? String(open) : ui("none")}
          tone={open ? "warning" : undefined}
        />
        {doctor && (
          <>
            <Fact
              label={ui("System problems")}
              value={problems ? String(problems) : ui("none")}
              tone={problems ? "warning" : undefined}
            />
            <Fact label={ui("System warnings")} value={warnings ? String(warnings) : ui("none")} />
          </>
        )}
      </dl>
      {doctor && (
        <p className="border-t border-border/60 px-4 py-2 text-xs text-muted-foreground">
          {ui("System checked {when}", {
            when: formatRelativeTime(doctor.checked_at, displayLocale),
          })}
        </p>
      )}
    </HomeCard>
  );
}

export function HomeSandboxes({ stats }: { stats: SandboxFleetStats }) {
  const ui = useUiTranslations();
  const live = stats.live_by_provider.reduce((sum, item) => sum + item.count, 0);
  const hours = Math.round(stats.running_seconds_in_window / 3600);
  return (
    <HomeCard
      title={ui("Sandboxes")}
      icon={<SandboxIcon className="size-4 text-muted-foreground" aria-hidden="true" />}
      link={{ href: "/sandboxes", label: ui("Open fleet") }}
      testId="home-sandboxes"
    >
      <dl className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-2 px-4 py-3">
        <Fact label={ui("Live now")} value={String(live)} />
        <Fact
          label={ui("Need attention")}
          value={String(stats.needs_attention)}
          tone={stats.needs_attention ? "warning" : undefined}
        />
        <Fact
          label={ui("Running time, {days} days", { days: stats.window_days })}
          value={`${hours} h`}
        />
        <Fact
          label={ui("Created, {days} days", { days: stats.window_days })}
          value={String(stats.created_in_window)}
        />
        <Fact
          label={ui("Rebuilt after loss, {days} days", { days: stats.window_days })}
          value={String(stats.recoveries_in_window)}
        />
      </dl>
    </HomeCard>
  );
}

/* ──────────────────────────── Get started ──────────────────────────── */

export function HomeGetStarted({
  hasModels,
  hasAgent,
  hasChannel,
}: {
  hasModels: boolean;
  hasAgent: boolean;
  hasChannel: boolean;
}) {
  const ui = useUiTranslations();
  const steps = [
    {
      done: hasModels,
      title: ui("Connect a model provider"),
      body: ui("Agents need at least one enabled model."),
      action: (
        <Link
          href="/models/providers"
          className={buttonVariants({ variant: "outline", size: "sm" })}
        >
          {ui("Add provider")}
        </Link>
      ),
    },
    {
      done: hasAgent,
      title: ui("Create your first agent"),
      body: ui("Describe it in a sentence, start from an example, or import one."),
      action: (
        <NewAgentLink>
          <Button variant="accent" size="sm">
            <Plus className="size-3.5" />
            {ui("New agent")}
          </Button>
        </NewAgentLink>
      ),
    },
    {
      done: hasChannel,
      title: ui("Publish a channel"),
      body: ui("Slack, a public chat page, the Agent API or a schedule."),
      action: (
        <Link
          href="/agents?view=channels"
          className={buttonVariants({ variant: "outline", size: "sm" })}
        >
          {ui("See channels")}
        </Link>
      ),
    },
  ];
  const done = steps.filter((step) => step.done).length;
  return (
    <HomeCard
      title={ui("Get started")}
      badges={
        <span className="text-xs text-muted-foreground">
          {ui("{done} of {total} done", { done, total: steps.length })}
        </span>
      }
      testId="home-get-started"
    >
      <ol className="divide-y divide-border/60">
        {steps.map((step, index) => (
          <li key={step.title} className="flex items-center gap-3.5 px-4 py-3">
            <span
              className={cn(
                "inline-flex size-7 shrink-0 items-center justify-center border text-xs font-semibold",
                step.done && "border-success/30 bg-success/10 text-success",
              )}
            >
              {step.done ? <Check className="size-3.5" aria-label={ui("Done")} /> : index + 1}
            </span>
            <div className="min-w-0 flex-1">
              <p
                className={cn(
                  "text-sm font-semibold",
                  step.done && "text-muted-foreground line-through decoration-border",
                )}
              >
                {step.title}
              </p>
              <p className="text-[13px] text-muted-foreground">{step.body}</p>
            </div>
            {!step.done && step.action}
          </li>
        ))}
      </ol>
    </HomeCard>
  );
}
