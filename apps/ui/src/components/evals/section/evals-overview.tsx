"use client";

// Evals Overview tab (approved mockup 01): where every agent stands on its
// datasets, and what needs a look. Live quality from observers joins this view
// with Observers v2; the Eval Builder's suggestions join with that agent.

import Link from "next/link";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
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
import { formatRelativeTime } from "@/lib/formatting";
import type { HealthIssue } from "@/lib/api/types";
import type { EvalsOverview } from "../evals-overview-data";
import { EXPERIMENT_WINDOW_DAYS } from "../evals-overview-data";
import { formatPercent } from "../experiment-format";
import { Sparkline } from "../sparkline";
import { FindingsList } from "./findings-list";

function PassRateCell({ rate }: { rate: number | null }) {
  if (rate === null) return <span className="text-muted-foreground">—</span>;
  return (
    <span className="inline-flex flex-col gap-1">
      <span className="tabular-nums">{formatPercent(rate)}</span>
      <span className="block h-1.5 w-24 bg-muted" aria-hidden>
        <span
          className="block h-full bg-foreground"
          style={{ width: `${Math.round(rate * 100)}%` }}
        />
      </span>
    </span>
  );
}

export function EvalsOverviewPanel({
  overview,
  findings,
}: {
  overview: EvalsOverview;
  findings: HealthIssue[];
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const withoutEvals = Math.max(0, overview.totalAgents - overview.agentsWithEvals);

  return (
    <div className="space-y-4">
      <StatGrid>
        <StatCard
          label={ui("Agents with evals")}
          value={ui("{count} of {total}", {
            count: overview.agentsWithEvals,
            total: overview.totalAgents,
          })}
          hint={
            withoutEvals === 0
              ? ui("Every agent has a dataset")
              : withoutEvals === 1
                ? ui("1 agent has no dataset")
                : ui("{count} agents have no dataset", { count: withoutEvals })
          }
        />
        <StatCard
          label={ui("Pass rate · last runs")}
          value={formatPercent(overview.passRate)}
          hint={ui("{count} cases", { count: overview.passRateCases })}
        />
        <StatCard
          label={ui("Experiments · {days} days", {
            days: EXPERIMENT_WINDOW_DAYS,
          })}
          value={overview.recentExperiments}
          hint={
            overview.runningExperiments > 0
              ? ui("{count} running", { count: overview.runningExperiments })
              : ui("none running")
          }
        />
        <StatCard
          label={ui("Open findings")}
          value={overview.openFindings}
          hint={ui("{count} high", { count: overview.highFindings })}
        />
      </StatGrid>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{ui("Agents")}</CardTitle>
          </CardHeader>
          <CardContent className="p-0">
            {overview.rows.length === 0 ? (
              <p className="px-4 py-6 text-sm text-muted-foreground">
                {ui("No agent has a dataset yet. Create one to start testing an agent.")}
              </p>
            ) : (
              <div className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{ui("Agent")}</TableHead>
                      <TableHead>{ui("Datasets")}</TableHead>
                      <TableHead>{ui("Pass rate")}</TableHead>
                      <TableHead>{ui("Trend")}</TableHead>
                      <TableHead>{ui("Findings")}</TableHead>
                      <TableHead>{ui("Last activity")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {overview.rows.map((row) => (
                      <TableRow key={row.agentId}>
                        <TableCell>
                          <Link
                            href={`/agents/${row.agentId}?tab=evals`}
                            className="font-semibold hover:underline"
                          >
                            {row.name}
                          </Link>
                        </TableCell>
                        <TableCell className="tabular-nums">{row.datasets.length}</TableCell>
                        <TableCell>
                          <PassRateCell rate={row.passRate} />
                        </TableCell>
                        <TableCell>
                          <Sparkline
                            values={row.trend}
                            label={ui("Pass rate over the last runs")}
                          />
                        </TableCell>
                        <TableCell>
                          {row.openFindings > 0 ? (
                            <span className="bg-warning/15 px-1.5 py-0.5 text-xs">
                              {ui("{count} open", { count: row.openFindings })}
                            </span>
                          ) : (
                            <span className="text-muted-foreground">{ui("None")}</span>
                          )}
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {row.lastActivityAt
                            ? formatRelativeTime(row.lastActivityAt, displayLocale)
                            : "—"}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{ui("Needs a look")}</CardTitle>
          </CardHeader>
          <CardContent className="p-0">
            <FindingsList
              issues={findings}
              limit={5}
              emptyText={ui("Nothing needs a look. Detectors and classifiers found no problems.")}
            />
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
