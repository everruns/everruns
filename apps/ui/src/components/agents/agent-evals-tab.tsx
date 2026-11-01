"use client";

// An agent's Evals tab (approved mockup 03): how this agent does on its
// datasets, its experiments, and what detectors found in its sessions.
// Datasets are evals whose target is a session on this agent. Live quality
// joins with Observers v2 and the Eval Builder's proposals with that agent.

import { useMemo, useState } from "react";
import Link from "next/link";
import { Play, Plus } from "lucide-react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useAllEvalExperiments, useEvalRunsMany, useEvals, useObservers } from "@/hooks";
import { useEvalFindings } from "@/hooks/use-health-issues";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { StatCard, StatGrid } from "@/components/layout";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatDate, formatRelativeTime } from "@/lib/formatting";
import { isArchivedStatus } from "@/lib/entity-lifecycle";
import type { EvalExperiment, EvalRun } from "@/lib/api/types";
import { evalAgentId, isTrendRun, weightedPassRate } from "@/components/evals/evals-overview-data";
import { formatPercent } from "@/components/evals/experiment-format";
import { isFinding } from "@/components/evals/findings";
import { FindingsList } from "@/components/evals/section/findings-list";
import { RunExperimentDialog } from "@/components/evals/run-experiment-dialog";

/** Finished runs across the agent's datasets, oldest first, for the chart. */
export function passRateSeries(runsByEval: Map<string, EvalRun[]>, names: Map<string, string>) {
  return [...runsByEval.entries()]
    .flatMap(([evalId, runs]) =>
      runs.filter(isTrendRun).map((run) => ({
        at: run.completed_at ?? run.created_at,
        dataset: names.get(evalId) ?? evalId,
        passRate: Math.round(run.summary!.pass_rate * 100),
      })),
    )
    .sort((a, b) => a.at.localeCompare(b.at))
    .slice(-30);
}

/** Whether every point of the series falls on one calendar day. */
export function sameDay(series: { at: string }[]): boolean {
  if (series.length === 0) return true;
  const first = new Date(series[0].at).toDateString();
  return series.every((point) => new Date(point.at).toDateString() === first);
}

/** A time of day when the chart covers one day, otherwise the date. */
function chartTick(at: string, oneDay: boolean, locale: string): string {
  const date = new Date(at);
  return oneDay
    ? date.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" })
    : date.toLocaleDateString(locale, { month: "short", day: "numeric" });
}

function ExperimentLinks({ experiments }: { experiments: EvalExperiment[] }) {
  const { displayLocale } = useLocale();
  return (
    <>
      {experiments.map((experiment) => (
        <li key={experiment.id}>
          <Link
            href={`/evals/${experiment.eval_id}/experiments/${experiment.id}`}
            className="block px-4 py-2.5 transition-colors hover:bg-muted/50"
          >
            <span className="block text-sm font-medium">{experiment.name}</span>
            <span className="block text-xs text-muted-foreground">
              {experiment.status} · {formatRelativeTime(experiment.created_at, displayLocale)}
            </span>
          </Link>
        </li>
      ))}
    </>
  );
}

export function AgentEvalsTab({ agentId }: { agentId: string }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const observersEnabled = useFeatureFlag("observers");
  const [runOpen, setRunOpen] = useState(false);
  const { data: evals } = useEvals({ includeArchived: false });
  const { data: findingsPage } = useEvalFindings();
  const { data: allExperiments } = useAllEvalExperiments();
  const { data: observers } = useObservers({
    includeArchived: false,
    enabled: observersEnabled,
  });

  const datasets = useMemo(
    () => (evals ?? []).filter((ev) => !isArchivedStatus(ev.status) && evalAgentId(ev) === agentId),
    [evals, agentId],
  );
  const runsByEval = useEvalRunsMany(datasets.map((ev) => ev.id));
  const names = useMemo(() => new Map(datasets.map((ev) => [ev.id, ev.name])), [datasets]);
  const experiments = useMemo(() => {
    const ids = new Set(datasets.map((ev) => ev.id));
    return (allExperiments ?? []).filter((e) => ids.has(e.eval_id)).slice(0, 5);
  }, [allExperiments, datasets]);
  const series = useMemo(() => passRateSeries(runsByEval, names), [runsByEval, names]);
  const findings = (findingsPage?.data ?? []).filter(
    (issue) => isFinding(issue) && issue.agent_id === agentId,
  );
  const watching = (observers ?? []).filter((observer) =>
    (observer.match.agent_ids ?? []).includes(agentId),
  );

  const pass = weightedPassRate(datasets);
  let everyTrial = 0;
  let cases = 0;
  for (const ev of datasets) {
    const summary = ev.last_run?.summary;
    if (!summary?.cases) continue;
    everyTrial += summary.cases_passed_every_trial ?? 0;
    cases += summary.cases;
  }
  const trials = datasets.find((ev) => ev.last_run?.summary?.trials)?.last_run?.summary?.trials;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap justify-end gap-2">
        <LinkButton variant="outline" href={`/evals/new?agent=${encodeURIComponent(agentId)}`}>
          <Plus className="size-4" />
          {ui("New dataset")}
        </LinkButton>
        <Button variant="accent" onClick={() => setRunOpen(true)} disabled={datasets.length === 0}>
          <Play className="size-4" />
          {ui("Run experiment")}
        </Button>
      </div>

      <StatGrid>
        <StatCard
          label={ui("Test pass rate")}
          value={formatPercent(pass.rate)}
          hint={ui("{count} cases across {datasets} datasets", {
            count: pass.cases,
            datasets: datasets.length,
          })}
        />
        <StatCard
          label={ui("Passes every trial")}
          value={formatPercent(cases > 0 ? everyTrial / cases : null)}
          hint={
            !trials
              ? ui("No runs yet")
              : trials === 1
                ? ui("1 trial per case")
                : ui("{count} trials per case", { count: trials })
          }
        />
        <StatCard
          label={ui("Observed by")}
          value={observersEnabled ? watching.length : "—"}
          hint={
            watching.length === 1
              ? ui("1 observer")
              : ui("{count} observers", { count: watching.length })
          }
        />
        <StatCard
          label={ui("Open findings")}
          value={findings.length}
          hint={findings[0]?.title ?? ui("None")}
        />
      </StatGrid>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">{ui("Pass rate over recent runs")}</CardTitle>
            </CardHeader>
            <CardContent>
              {series.length < 2 ? (
                <p className="py-6 text-sm text-muted-foreground">
                  {ui("The chart appears after two finished runs.")}
                </p>
              ) : (
                <div className="h-48">
                  <ResponsiveContainer width="100%" height="100%">
                    <LineChart data={series} margin={{ top: 8, right: 8, bottom: 0, left: -16 }}>
                      <CartesianGrid
                        strokeDasharray="3 3"
                        className="stroke-border"
                        vertical={false}
                      />
                      <XAxis
                        dataKey="at"
                        tickFormatter={(value: string) =>
                          chartTick(value, sameDay(series), displayLocale)
                        }
                        tick={{ fontSize: 11 }}
                      />
                      <YAxis domain={[0, 100]} tick={{ fontSize: 11 }} unit="%" />
                      <Tooltip
                        labelFormatter={(value) => formatDate(String(value), displayLocale)}
                        formatter={(value, _name, item) => [
                          `${value}%`,
                          (item?.payload as { dataset?: string } | undefined)?.dataset ?? "",
                        ]}
                      />
                      <Line
                        type="monotone"
                        dataKey="passRate"
                        stroke="currentColor"
                        strokeWidth={1.5}
                        dot={{ r: 2 }}
                        isAnimationActive={false}
                      />
                    </LineChart>
                  </ResponsiveContainer>
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm">{ui("Datasets")}</CardTitle>
            </CardHeader>
            <CardContent className="p-0">
              {datasets.length === 0 ? (
                <p className="px-4 py-6 text-sm text-muted-foreground">
                  {ui(
                    "This agent has no dataset yet. A dataset is a set of cases the agent should handle.",
                  )}
                </p>
              ) : (
                <div className="overflow-x-auto">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>{ui("Dataset")}</TableHead>
                        <TableHead>{ui("Cases")}</TableHead>
                        <TableHead>{ui("Last run")}</TableHead>
                        <TableHead>{ui("Result")}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {datasets.map((ev) => (
                        <TableRow key={ev.id}>
                          <TableCell>
                            <Link
                              href={`/evals/${ev.id}`}
                              className="font-semibold hover:underline"
                            >
                              {ev.name}
                            </Link>
                            {ev.version ? (
                              <span className="ml-1.5 text-xs text-muted-foreground">
                                v{ev.version}
                              </span>
                            ) : null}
                          </TableCell>
                          <TableCell className="tabular-nums">{ev.case_count ?? 0}</TableCell>
                          <TableCell className="text-muted-foreground">
                            {ev.last_run
                              ? formatRelativeTime(ev.last_run.created_at, displayLocale)
                              : ui("Never")}
                          </TableCell>
                          <TableCell className="tabular-nums">
                            {formatPercent(ev.last_run?.summary?.pass_rate)}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">{ui("Experiments")}</CardTitle>
            </CardHeader>
            <CardContent className="p-0">
              <ul className="divide-y">
                <ExperimentLinks experiments={experiments} />
              </ul>
              {experiments.length === 0 && (
                <p className="px-4 py-4 text-sm text-muted-foreground">
                  {ui("No experiments yet.")}
                </p>
              )}
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">{ui("Findings")}</CardTitle>
            </CardHeader>
            <CardContent className="p-0">
              <FindingsList
                issues={findings}
                showTime
                emptyText={ui("No open findings for this agent.")}
              />
            </CardContent>
          </Card>
        </div>
      </div>

      {runOpen && (
        <RunExperimentDialog
          open={runOpen}
          onOpenChange={setRunOpen}
          evals={datasets}
          defaultEvalId={datasets[0]?.id}
        />
      )}
    </div>
  );
}
