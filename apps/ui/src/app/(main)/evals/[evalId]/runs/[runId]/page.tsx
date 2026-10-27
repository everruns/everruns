"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { use, useCallback, useState } from "react";
import { useEval, useEvalRun, useCancelEvalRun, usePageTitle } from "@/hooks";
import Link from "next/link";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft, Square, ExternalLink, ChevronRight, ChevronDown } from "lucide-react";
import { TranscriptView } from "@/components/evals/transcript-view";
import { RunResultsMatrix } from "@/components/evals/run-results-matrix";
import { ShareRunButton } from "@/components/evals/share-run-button";
import { normalizeScores, targetLabel } from "@/components/evals/eval-format";
import type { EvalCaseResult, EvalRun, EvalRunStatus, EvalTarget } from "@/lib/api/types";

function passRateColor(rate: number): string {
  if (rate >= 0.9) return "text-green-600";
  if (rate >= 0.7) return "text-yellow-600";
  return "text-red-600";
}

function resultStatusVariant(status: string): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "passed":
      return "default";
    case "pending":
    case "running":
    case "skipped":
      return "secondary";
    case "failed":
    case "errored":
    case "timeout":
      return "destructive";
    default:
      return "outline";
  }
}

function runStatusVariant(
  status: EvalRunStatus,
): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "completed":
      return "default";
    case "running":
    case "pending":
      return "secondary";
    case "failed":
    case "cancelled":
      return "destructive";
    default:
      return "outline";
  }
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function TargetBadge({ target }: { target?: EvalTarget }) {
  if (!target) return null;
  return (
    <Badge variant="outline" className="text-xs">
      {targetLabel(target)}
    </Badge>
  );
}

function safeExternalLink(url: string | null | undefined): string | undefined {
  if (!url) return undefined;
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" || parsed.protocol === "http:" ? url : undefined;
  } catch {
    return undefined;
  }
}

// Attribution badge for an imported run: which external system produced it, its
// version, and a link back when one is provided.
function AttributionBadge({ run }: { run: EvalRun }) {
  if (run.source !== "external") return null;
  const system = run.attribution?.system ?? "external";
  const version = run.attribution?.version;
  const url = safeExternalLink(run.attribution?.url);
  const label = `via ${system}${version ? ` ${version}` : ""}`;
  const badge = (
    <Badge variant="secondary" className="text-xs">
      {label}
    </Badge>
  );
  return url ? (
    <a href={url} target="_blank" rel="noopener noreferrer" title={url}>
      {badge}
    </a>
  ) : (
    badge
  );
}

function ResultRow({ result, columns }: { result: EvalCaseResult; columns: number }) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(false);
  const scores = normalizeScores(result.scores);
  // Imported results carry a transcript/metrics envelope instead of a session;
  // only those get an expandable drill-down.
  const detail = result.metadata;
  const hasDetail = !!(detail?.transcript || detail?.metrics);

  return (
    <>
      <tr className="border-b last:border-b-0">
        <td className="py-3 px-4">
          <div className="flex items-center gap-2">
            {hasDetail && (
              <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                aria-expanded={open}
                aria-label={open ? ui("Hide transcript") : ui("Show transcript")}
                className="text-muted-foreground hover:text-foreground"
              >
                {open ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
              </button>
            )}
            <span className="text-sm font-medium">{result.case_name ?? result.eval_case_id}</span>
            {result.session_id && (
              <Link
                href={`/sessions/${result.session_id}/trace`}
                target="_blank"
                rel="noopener noreferrer"
                aria-label={ui("Open session for {value0} in new tab", {
                  value0: result.case_name ?? result.eval_case_id,
                })}
              >
                <ExternalLink className="w-3 h-3 text-muted-foreground hover:text-foreground" />
              </Link>
            )}
          </div>
        </td>
        <td className="py-3 px-4">
          <Badge variant={resultStatusVariant(result.status)}>{result.status}</Badge>
        </td>
        <td className="py-3 px-4 text-sm text-muted-foreground">
          <TargetBadge target={result.target_snapshot} />
        </td>
        <td className="py-3 px-4 text-sm text-muted-foreground">{result.turns ?? "-"}</td>
        <td className="py-3 px-4 text-sm text-muted-foreground">
          {result.latency_ms ? formatDuration(result.latency_ms) : "-"}
        </td>
        <td className="py-3 px-4 text-sm text-muted-foreground">
          {result.input_tokens != null ? result.input_tokens.toLocaleString() : "-"}
        </td>
        <td className="py-3 px-4 text-sm text-muted-foreground">
          {result.output_tokens != null ? result.output_tokens.toLocaleString() : "-"}
        </td>
        <td className="py-3 px-4">
          {scores.length > 0 && (
            <div className="flex flex-wrap gap-1">
              {scores.map((score, i) => (
                <Badge
                  key={`${score.name}-${i}`}
                  variant={score.pass ? "default" : "destructive"}
                  className="text-xs"
                  title={score.reason}
                >
                  {score.name}: {score.value.toFixed(1)}
                </Badge>
              ))}
            </div>
          )}
          {result.error_message && (
            <p className="text-xs text-destructive mt-1">{result.error_message}</p>
          )}
        </td>
      </tr>
      {open && hasDetail && (
        <tr className="border-b last:border-b-0 bg-muted/20">
          <td colSpan={columns} className="px-4 pb-4">
            <TranscriptView transcript={detail?.transcript} metrics={detail?.metrics} />
          </td>
        </tr>
      )}
    </>
  );
}

export default function EvalRunDetailPage({
  params,
}: {
  params: Promise<{ evalId: string; runId: string }>;
}) {
  const ui = useUiTranslations();
  const { evalId, runId } = use(params);
  const { data: ev } = useEval(evalId);
  const { data: run, isLoading: runLoading } = useEvalRun(evalId, runId);
  usePageTitle(ui("Run"), ev?.name ?? null, ui("Eval"));
  const cancelRun = useCancelEvalRun(evalId);
  // null = follow the default (matrix when the run spans multiple targets);
  // a user choice sticks once made.
  const [view, setView] = useState<"table" | "matrix" | null>(null);
  useChatPageFilters({ tab: view });

  const handleCancel = useCallback(async () => {
    try {
      await cancelRun.mutateAsync(runId);
    } catch (error) {
      console.error("Failed to cancel run:", error);
    }
  }, [cancelRun, runId]);

  if (runLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!run) {
    return (
      <ResourceNotFound
        title={ui("Run not found")}
        description={ui(
          "This eval run may have been deleted, moved to another organization, or the URL may be wrong.",
        )}
        backHref={`/evals/${evalId}`}
        backLabel="Back to eval"
        resourceId={runId}
      />
    );
  }

  const canCancel = run.status === "pending" || run.status === "running";

  // The matrix is the natural view when one run spans several targets (a
  // published Mira run); otherwise the flat table is clearer.
  const targetCount = new Set(run.results.map((r) => targetLabel(r.target_snapshot))).size;
  const multiTarget = targetCount > 1;
  const effectiveView = view ?? (multiTarget ? "matrix" : "table");

  return (
    <div className="container mx-auto p-6">
      <Link
        href={`/evals/${evalId}`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        {ui("Back to")} {ev?.name ?? ui("Eval")}
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            {ui("Run")}
            <Badge variant={runStatusVariant(run.status)}>{run.status}</Badge>
            <AttributionBadge run={run} />
            {run.target && <TargetBadge target={run.target} />}
          </h1>
          <div className="flex items-center gap-4 mt-2 text-sm text-muted-foreground">
            <span>
              {ui("Started:")}{" "}
              {run.started_at ? new Date(run.started_at).toLocaleString() : ui("Pending")}
            </span>
            {run.completed_at && (
              <span>
                {ui("Completed:")} {new Date(run.completed_at).toLocaleString()}
              </span>
            )}
            <span>
              {ui("Triggered by:")} {run.triggered_by}
            </span>
            {run.model_override && (
              <span>
                {ui("Model:")} {run.model_override}
              </span>
            )}
          </div>
        </div>
        <div className="flex items-start gap-3">
          <ShareRunButton evalId={evalId} runId={runId} />
          {canCancel && (
            <Button variant="destructive" onClick={handleCancel} disabled={cancelRun.isPending}>
              <Square className="w-4 h-4 mr-2" />
              {cancelRun.isPending ? ui("Cancelling...") : ui("Cancel Run")}
            </Button>
          )}
        </div>
      </div>

      {/* Summary cards */}
      {run.summary && (
        <div className="grid gap-4 md:grid-cols-4 mb-6">
          <Card>
            <CardContent className="pt-4">
              <p className="text-xs text-muted-foreground">{ui("Pass Rate")}</p>
              <p className={`text-2xl font-bold ${passRateColor(run.summary.pass_rate)}`}>
                {(run.summary.pass_rate * 100).toFixed(1)}%
              </p>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4">
              <p className="text-xs text-muted-foreground">{ui("Results")}</p>
              <p className="text-2xl font-bold">
                {run.summary.passed}/{run.summary.total}
              </p>
              <p className="text-xs text-muted-foreground">
                {run.summary.failed} {ui("failed,")} {run.summary.errored} {ui("errored")}
              </p>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4">
              <p className="text-xs text-muted-foreground">{ui("Avg Latency")}</p>
              <p className="text-2xl font-bold">{formatDuration(run.summary.avg_latency_ms)}</p>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4">
              <p className="text-xs text-muted-foreground">{ui("Total Tokens")}</p>
              <p className="text-2xl font-bold">
                {(
                  run.summary.total_input_tokens + run.summary.total_output_tokens
                ).toLocaleString()}
              </p>
              <p className="text-xs text-muted-foreground">
                {run.summary.total_input_tokens.toLocaleString()} {ui("in /")}{" "}
                {run.summary.total_output_tokens.toLocaleString()} {ui("out")}
              </p>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Results */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>
            {ui("Results (")}
            {run.results.length})
          </CardTitle>
          {multiTarget && run.results.length > 0 && (
            <div className="flex gap-1">
              <Button
                variant={effectiveView === "matrix" ? "secondary" : "ghost"}
                size="sm"
                onClick={() => setView("matrix")}
              >
                {ui("Matrix")}
              </Button>
              <Button
                variant={effectiveView === "table" ? "secondary" : "ghost"}
                size="sm"
                onClick={() => setView("table")}
              >
                {ui("Table")}
              </Button>
            </div>
          )}
        </CardHeader>
        <CardContent>
          {run.results.length === 0 ? (
            <p className="text-center py-8 text-muted-foreground">
              {canCancel ? ui("Waiting for results...") : ui("No results")}
            </p>
          ) : effectiveView === "matrix" ? (
            <RunResultsMatrix results={run.results} />
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left">
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Case")}
                    </th>
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Status")}
                    </th>
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Target")}
                    </th>
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Turns")}
                    </th>
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Latency")}
                    </th>
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Input")}
                    </th>
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Output")}
                    </th>
                    <th className="py-2 px-4 text-sm font-medium text-muted-foreground">
                      {ui("Scores")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {run.results.map((result) => (
                    <ResultRow key={result.id} result={result} columns={8} />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
