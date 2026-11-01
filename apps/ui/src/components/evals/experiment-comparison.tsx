"use client";

// The experiment page body: variants side by side, the verdict, and cases
// compared one by one with changed cases first (approved mockup 02).

import { useMemo, useState } from "react";
import { useUiTranslations } from "@/providers/locale-provider";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Notice } from "@/components/ui/notice";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatCostUsd, formatDuration } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import type { EvalExperiment, ExperimentVariant, VariantVerdict } from "@/lib/api/types";
import {
  costPerCase,
  deltaTone,
  formatPercent,
  formatPoints,
  passedEveryTrial,
  relativeChange,
  verdictDetail,
  verdictHeadline,
} from "./experiment-format";

function Delta({
  value,
  suffix = "",
  higherIsBetter = true,
}: {
  value: number | null;
  suffix?: string;
  higherIsBetter?: boolean;
}) {
  if (value === null || Math.round(value) === 0) return null;
  const tone = deltaTone(value, higherIsBetter);
  return (
    <span
      className={cn(
        "ml-1.5 text-xs font-medium tabular-nums",
        tone === "good" && "text-success",
        tone === "bad" && "text-destructive",
      )}
    >
      {formatPoints(value)}
      {suffix}
    </span>
  );
}

function RateBar({ rate }: { rate: number }) {
  return (
    <span className="ml-2 inline-block h-1.5 w-20 bg-muted align-middle" aria-hidden>
      <span
        className="block h-full bg-foreground"
        style={{ width: `${Math.round(rate * 100)}%` }}
      />
    </span>
  );
}

function variantSetup(variant: ExperimentVariant, modelNames: Map<string, string>): string | null {
  const target = variant.target;
  const model =
    variant.model_override ?? (target?.type === "session" ? target.model_id : undefined);
  if (model) return modelNames.get(model) ?? model;
  return null;
}

export function VariantTable({
  experiment,
  modelNames,
}: {
  experiment: EvalExperiment;
  modelNames: Map<string, string>;
}) {
  const ui = useUiTranslations();
  const base = experiment.variants[0];
  const basePass = base?.summary?.pass_rate ?? null;
  const baseEvery = passedEveryTrial(base?.summary);
  const baseCost = base ? costPerCase(base) : null;

  return (
    <div className="overflow-x-auto border bg-card">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{ui("Variant")}</TableHead>
            <TableHead>{ui("Pass rate")}</TableHead>
            <TableHead>{ui("Passes every trial")}</TableHead>
            <TableHead>{ui("Score")}</TableHead>
            <TableHead>{ui("Cost per case")}</TableHead>
            <TableHead>{ui("Time per case")}</TableHead>
            <TableHead>{ui("Turns")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {experiment.variants.map((variant, index) => {
            const summary = variant.summary;
            const isBase = index === 0;
            const setup = variantSetup(variant, modelNames);
            const every = passedEveryTrial(summary);
            const cost = costPerCase(variant);
            return (
              <TableRow key={variant.label}>
                <TableCell>
                  <span className="font-semibold">{variant.label}</span>
                  <span className="ml-1.5 text-xs text-muted-foreground">
                    {[isBase ? ui("baseline") : null, setup].filter(Boolean).join(" · ")}
                  </span>
                  {variant.status !== "completed" && (
                    <Badge variant="outline" className="ml-2 text-xs">
                      {variant.status}
                    </Badge>
                  )}
                </TableCell>
                <TableCell className="tabular-nums">
                  {summary ? (
                    <>
                      {formatPercent(summary.pass_rate)}
                      <RateBar rate={summary.pass_rate} />
                      {!isBase && basePass !== null && (
                        <Delta value={(summary.pass_rate - basePass) * 100} />
                      )}
                    </>
                  ) : (
                    "—"
                  )}
                </TableCell>
                <TableCell className="tabular-nums">
                  {formatPercent(every)}
                  {!isBase && every !== null && baseEvery !== null && (
                    <Delta value={(every - baseEvery) * 100} />
                  )}
                </TableCell>
                <TableCell className="tabular-nums">
                  {summary ? summary.avg_score.toFixed(2) : "—"}
                </TableCell>
                <TableCell className="tabular-nums">
                  {cost === null ? "—" : formatCostUsd(cost)}
                  {!isBase && (
                    <Delta
                      value={relativeChange(cost, baseCost)}
                      suffix="%"
                      higherIsBetter={false}
                    />
                  )}
                </TableCell>
                <TableCell className="tabular-nums">
                  {summary ? formatDuration(Math.round(summary.avg_latency_ms)) : "—"}
                </TableCell>
                <TableCell className="tabular-nums">
                  {summary ? summary.avg_turns.toFixed(1) : "—"}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

export function VerdictNotice({ verdict }: { verdict: VariantVerdict }) {
  const ui = useUiTranslations();
  const variant =
    verdict.verdict === "improvement"
      ? "success"
      : verdict.verdict === "regression"
        ? "destructive"
        : "warning";
  return (
    <Notice variant={variant} data-testid="experiment-verdict">
      <p className="text-sm">
        <strong>{verdictHeadline(verdict.verdict, ui)}</strong> {verdictDetail(verdict, ui)}
      </p>
    </Notice>
  );
}

type CaseFilter = "changed" | "all";

export function CaseComparisonTable({ experiment }: { experiment: EvalExperiment }) {
  const ui = useUiTranslations();
  const cases = useMemo(() => experiment.comparison?.cases ?? [], [experiment.comparison]);
  const changed = cases.filter((c) => c.changed).length;
  const [filter, setFilter] = useState<CaseFilter>(changed > 0 ? "changed" : "all");
  const visible = filter === "changed" ? cases.filter((c) => c.changed) : cases;
  const labels = experiment.variants.map((v) => v.label);

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between gap-3 space-y-0">
        <CardTitle className="text-sm">
          {ui("Cases")}
          <span className="ml-1.5 font-normal text-muted-foreground">
            {changed > 0
              ? ui("{count} changed, shown first", { count: changed })
              : ui("none changed")}
          </span>
        </CardTitle>
        <SegmentedControl
          label={ui("Show")}
          value={filter}
          onValueChange={(value) => setFilter(value as CaseFilter)}
          options={[
            { value: "changed", label: ui("Changed") },
            { value: "all", label: ui("All") },
          ]}
        />
      </CardHeader>
      <CardContent className="p-0">
        {visible.length === 0 ? (
          <p className="px-4 py-6 text-sm text-muted-foreground">
            {cases.length === 0
              ? ui("Results appear here as cases finish.")
              : ui("No case changed between variants.")}
          </p>
        ) : (
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{ui("Case")}</TableHead>
                  {labels.map((label) => (
                    <TableHead key={label}>{label}</TableHead>
                  ))}
                  {labels.map((label) => (
                    <TableHead key={`${label}-cost`}>{ui("{label} cost", { label })}</TableHead>
                  ))}
                  <TableHead>
                    <span className="sr-only">{ui("Changed")}</span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {visible.map((row) => (
                  <TableRow key={row.case_id} className={cn(row.changed && "bg-warning/5")}>
                    <TableCell>{row.case_name}</TableCell>
                    {row.cells.map((cell, index) => {
                      const base = row.cells[0];
                      const better =
                        index > 0 && base && cell.trials > 0 && base.trials > 0
                          ? cell.passed / cell.trials - base.passed / base.trials
                          : 0;
                      return (
                        <TableCell
                          key={cell.label}
                          className={cn(
                            "tabular-nums",
                            index > 0 && "font-semibold",
                            better > 0 && "text-success",
                            better < 0 && "text-destructive",
                          )}
                        >
                          {cell.trials > 0 ? `${cell.passed}/${cell.trials}` : "—"}
                        </TableCell>
                      );
                    })}
                    {row.cells.map((cell) => (
                      <TableCell
                        key={`${cell.label}-cost`}
                        className="tabular-nums text-muted-foreground"
                      >
                        {formatCostUsd(cell.cost_usd)}
                      </TableCell>
                    ))}
                    <TableCell className="text-xs text-muted-foreground">
                      {row.changed ? ui("Changed") : null}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
