"use client";

// Public, read-only view of a shared eval run. Lives OUTSIDE the (main) route
// group, so it renders without login. It fetches the sanitized public endpoint
// (no auth) and reuses the same matrix/transcript components as the authed view.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { use, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { getPublicEvalRun } from "@/lib/api/evals";
import { RunResultsMatrix } from "@/components/evals/run-results-matrix";
import { TranscriptView } from "@/components/evals/transcript-view";
import { normalizeScores, targetLabel } from "@/components/evals/eval-format";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ChevronRight, ChevronDown } from "lucide-react";
import type { EvalCaseResult, PublicEvalCaseResult, PublicEvalRun } from "@/lib/api/types";

function resultStatusVariant(status: string): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "passed":
      return "default";
    case "failed":
    case "errored":
    case "timeout":
      return "destructive";
    default:
      return "secondary";
  }
}

// Map the public result shape onto the fields RunResultsMatrix reads (it only
// uses case_name/eval_case_id/target_snapshot/status/scores).
function toMatrixResults(pub: PublicEvalRun): EvalCaseResult[] {
  return pub.results.map((r, i) => ({
    id: String(i),
    eval_case_id: r.case_name ?? `case-${i}`,
    case_name: r.case_name,
    target_snapshot: r.target,
    status: r.status,
    scores: r.scores,
    created_at: pub.created_at,
    updated_at: pub.created_at,
  }));
}

function PublicResultRow({ result, index }: { result: PublicEvalCaseResult; index: number }) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(false);
  const scores = normalizeScores(result.scores);
  const hasDetail = !!(result.transcript || result.metrics);
  return (
    <div className="border-b last:border-b-0 py-3">
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
        <span className="text-sm font-medium">
          {result.case_name ?? ui("case {value0}", { value0: index + 1 })}
        </span>
        <Badge variant={resultStatusVariant(result.status)}>{result.status}</Badge>
        {result.target && (
          <Badge variant="outline" className="text-xs">
            {targetLabel(result.target)}
          </Badge>
        )}
        <div className="flex flex-wrap gap-1">
          {scores.map((s, i) => (
            <Badge
              key={`${s.name}-${i}`}
              variant={s.pass ? "default" : "destructive"}
              className="text-xs"
              title={s.reason}
            >
              {s.name}: {s.value.toFixed(1)}
            </Badge>
          ))}
        </div>
      </div>
      {open && hasDetail && (
        <div className="mt-2">
          <TranscriptView transcript={result.transcript} metrics={result.metrics} />
        </div>
      )}
    </div>
  );
}

export default function SharedEvalRunPage({ params }: { params: Promise<{ token: string }> }) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const { token } = use(params);
  const { data, isLoading, isError } = useQuery({
    queryKey: ["public-eval-run", token],
    queryFn: () => getPublicEvalRun(token),
    retry: false,
  });

  if (isLoading) {
    return (
      <div className="container mx-auto max-w-4xl p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (isError || !data) {
    return (
      <div className="container mx-auto max-w-4xl p-6 text-center">
        <h1 className="text-xl font-semibold mb-2">{ui("Shared run not found")}</h1>
        <p className="text-muted-foreground">
          {ui("This share link is invalid, revoked, or expired.")}
        </p>
      </div>
    );
  }

  const targetCount = new Set(data.results.map((r) => targetLabel(r.target))).size;
  const attribution = data.attribution;

  return (
    <div className="container mx-auto max-w-4xl p-6">
      <div className="mb-6">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold">{ui("Eval run")}</h1>
          <Badge variant={data.status === "completed" ? "default" : "secondary"}>
            {data.status}
          </Badge>
          {data.source === "external" && attribution && (
            <Badge variant="secondary" className="text-xs">
              {ui("via")} {attribution.system ?? ui("external")}
              {attribution.version ? ui(" {value0}", { value0: attribution.version }) : ""}
            </Badge>
          )}
        </div>
        <p className="mt-1 text-sm text-muted-foreground">
          {ui("Read-only shared view ·")} {new Date(data.created_at).toLocaleString(displayLocale)}
        </p>
      </div>

      {data.summary && (
        <div className="grid gap-4 md:grid-cols-3 mb-6">
          <Card>
            <CardContent className="pt-4">
              <p className="text-xs text-muted-foreground">{ui("Pass Rate")}</p>
              <p className="text-2xl font-bold">{(data.summary.pass_rate * 100).toFixed(1)}%</p>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4">
              <p className="text-xs text-muted-foreground">{ui("Results")}</p>
              <p className="text-2xl font-bold">
                {data.summary.passed}/{data.summary.total}
              </p>
              <p className="text-xs text-muted-foreground">
                {data.summary.failed} {ui("failed,")} {data.summary.errored} {ui("errored")}
              </p>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4">
              <p className="text-xs text-muted-foreground">{ui("Total Tokens")}</p>
              <p className="text-2xl font-bold">
                {(
                  data.summary.total_input_tokens + data.summary.total_output_tokens
                ).toLocaleString(displayLocale)}
              </p>
            </CardContent>
          </Card>
        </div>
      )}

      {targetCount > 1 && (
        <Card className="mb-6">
          <CardHeader>
            <CardTitle>{ui("Matrix")}</CardTitle>
          </CardHeader>
          <CardContent>
            <RunResultsMatrix results={toMatrixResults(data)} />
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>
            {ui("Results (")}
            {data.results.length})
          </CardTitle>
        </CardHeader>
        <CardContent>
          {data.results.length === 0 ? (
            <p className="text-center py-8 text-muted-foreground">{ui("No results")}</p>
          ) : (
            data.results.map((r, i) => <PublicResultRow key={i} result={r} index={i} />)
          )}
        </CardContent>
      </Card>
    </div>
  );
}
