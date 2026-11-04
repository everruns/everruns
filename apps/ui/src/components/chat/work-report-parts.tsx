/**
 * Decisions:
 * - These render a server-built work report; they never derive progress from
 *   events themselves (see `use-turn-work-report.ts`).
 * - The plan is pinned under the work log header while the turn runs, outside
 *   the fold, because "step 3 of 4" is the answer to "how far along is it".
 *   No plan, no box and no bar: a bar without a declared plan would be made up.
 * - The done box appears only when it says more than "Worked for 13s": files
 *   changed, tasks started, artifacts, what the agent checked, a failure it
 *   reported, or a summary the agent wrote itself (`report_done`). A
 *   summary taken from the reply is not repeated, since the reply is right
 *   below it.
 */
"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import {
  Check,
  CheckCircle2,
  Circle,
  CircleAlert,
  FileText,
  Link2,
  LoaderCircle,
  Minus,
} from "lucide-react";
import type { WorkPlan, WorkReport } from "@/lib/api/types";
import { formatWorkFact } from "@/lib/i18n";
import { cn } from "@/lib/utils";

type CountedFact = "files_changed" | "tasks_started";
const SHOWN_FACTS: readonly CountedFact[] = ["files_changed", "tasks_started"];
const MAX_ARTIFACT_CHIPS = 6;

export function WorkPlanPinned({ plan }: { plan: WorkPlan }) {
  const { t } = useLocale();
  if (plan.total === 0) return null;
  const current = Math.min(plan.done + 1, plan.total);
  const finished = plan.done >= plan.total;
  const title = finished
    ? t("work_plan_step", { current: plan.total, total: plan.total })
    : t("work_plan_step", { current, total: plan.total });
  return (
    <section
      aria-label={t("work_plan")}
      className="rounded-md border border-border/70 bg-card/40 px-3 py-2.5"
      data-testid="work-plan"
    >
      <div className="flex items-baseline justify-between gap-3 text-sm">
        <span className="min-w-0 truncate font-medium">
          {title}
          {plan.current && !finished ? (
            <span className="text-muted-foreground"> · {plan.current}</span>
          ) : null}
        </span>
        <span className="shrink-0 text-xs text-muted-foreground">{t("work_plan")}</span>
      </div>
      <div className="mt-2 flex gap-1" aria-hidden>
        {plan.items.map((item, index) => (
          <span
            key={index}
            className={cn(
              "h-1 flex-1 rounded-full",
              item.status === "done" || item.status === "skipped"
                ? "bg-primary"
                : item.status === "in_progress"
                  ? "bg-primary/45"
                  : "bg-muted",
            )}
          />
        ))}
      </div>
      <ol className="mt-2 space-y-1 text-sm">
        {plan.items.map((item, index) => (
          <li key={index} className="flex items-start gap-2">
            <PlanItemIcon status={item.status} />
            <span
              className={cn(
                item.status === "pending" && "text-muted-foreground",
                item.status === "skipped" && "text-muted-foreground line-through",
              )}
            >
              {item.title}
            </span>
          </li>
        ))}
      </ol>
    </section>
  );
}

function PlanItemIcon({ status }: { status: WorkPlan["items"][number]["status"] }) {
  const className = "mt-0.5 size-3.5 shrink-0";
  switch (status) {
    case "done":
      return <Check className={cn(className, "text-muted-foreground")} aria-hidden />;
    case "in_progress":
      return (
        <LoaderCircle
          className={cn(className, "animate-spin text-primary motion-reduce:animate-none")}
          aria-hidden
        />
      );
    case "skipped":
      return <Minus className={cn(className, "text-muted-foreground")} aria-hidden />;
    default:
      return <Circle className={cn(className, "text-muted-foreground/60")} aria-hidden />;
  }
}

/** Facts worth a line ("2 files changed · 1 task started"); empty when none. */
export function workFactsLine(report: WorkReport, locale: string): string {
  const facts = report.result?.facts ?? [];
  return SHOWN_FACTS.flatMap((kind) => {
    const fact = facts.find((candidate) => candidate.kind === kind);
    return fact && fact.value > 0
      ? [formatWorkFact(locale === "uk" ? "uk" : "en", kind, fact.value)]
      : [];
  }).join(" · ");
}

export function WorkResultBox({ report }: { report: WorkReport }) {
  const ui = useUiTranslations();
  const { locale, t } = useLocale();
  if (report.state !== "done" || !report.result) return null;
  const facts = workFactsLine(report, locale);
  const summary = report.result.summary_source === "agent" ? report.result.summary : null;
  const checks = report.result.checks ?? null;
  const failed = report.result.outcome === "failed";
  const artifacts = report.result.artifacts ?? [];
  if (!facts && !summary && !checks && !failed && artifacts.length === 0) return null;
  const lead = failed ? t("work_failed") : t("work_done");

  return (
    <section
      className="rounded-md border border-border/70 bg-muted/30 px-3 py-2.5"
      data-testid="work-result"
    >
      <div className="flex items-center gap-2 text-sm font-medium">
        {failed ? (
          <CircleAlert className="size-4 shrink-0 text-destructive" aria-hidden />
        ) : (
          <CheckCircle2 className="size-4 shrink-0 text-success" aria-hidden />
        )}
        <span>{facts ? ui("{value0} · {value1}", { value0: lead, value1: facts }) : lead}</span>
      </div>
      {summary ? <p className="mt-1.5 pl-6 text-sm">{summary}</p> : null}
      {checks ? (
        <p className="mt-1 pl-6 text-xs text-muted-foreground">{t("work_checked", { checks })}</p>
      ) : null}
      {artifacts.length > 0 ? (
        <ul className="mt-2 flex flex-wrap gap-1.5 pl-6">
          {artifacts.slice(0, MAX_ARTIFACT_CHIPS).map((artifact, index) => {
            const Icon = artifact.url ? Link2 : FileText;
            const chip = (
              <>
                <Icon className="size-3.5 shrink-0" aria-hidden />
                <span className="truncate">{artifact.name}</span>
              </>
            );
            const className =
              "inline-flex max-w-64 items-center gap-1 rounded border border-border/70 bg-background px-2 py-0.5 text-xs";
            return (
              <li key={`${artifact.name}-${index}`}>
                {artifact.url && /^https?:\/\//i.test(artifact.url) ? (
                  <a
                    href={artifact.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className={cn(className, "hover:bg-muted")}
                    title={artifact.url}
                  >
                    {chip}
                  </a>
                ) : (
                  <span className={className} title={artifact.path ?? artifact.name}>
                    {chip}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      ) : null}
    </section>
  );
}

/** Live status for the work log header, localized where the report is structured. */
export function workReportStatus(
  report: WorkReport,
  t: (key: "work_needs_approval" | "work_waiting_answer" | "work_waiting_you") => string,
): string | undefined {
  if (report.state === "needs_you" && report.attention) {
    const lead =
      report.attention.kind === "approval"
        ? t("work_needs_approval")
        : report.attention.kind === "question"
          ? t("work_waiting_answer")
          : t("work_waiting_you");
    return report.attention.prompt ? `${lead} · ${report.attention.prompt}` : lead;
  }
  if (report.state !== "working") return undefined;
  // The headline names the running step (with its count) or the plan's current
  // item; with neither it is a bare "Working", which the header already says.
  const running = report.steps.some((step) => step.state === "running");
  return running || report.plan?.current ? report.headline : undefined;
}
