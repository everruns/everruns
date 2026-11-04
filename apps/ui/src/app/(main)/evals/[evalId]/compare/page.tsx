"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { use, Suspense } from "react";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { useEval, useEvalRunComparison, usePageTitle } from "@/hooks";
import { RunComparison } from "@/components/evals/run-comparison";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { MAX_COMPARE_RUNS, parseCompareRunIds } from "@/lib/eval-run-comparison";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft } from "lucide-react";

function CompareContent({ evalId }: { evalId: string }) {
  const ui = useUiTranslations();
  const searchParams = useSearchParams();
  const { runIds, ignoredCount, exceededLimit } = parseCompareRunIds(searchParams.get("runs"));

  const { data: ev } = useEval(evalId);
  usePageTitle(ui("Compare runs"), ev?.name ?? null, ui("Eval"));
  const { runs, isLoading, isError } = useEvalRunComparison(evalId, runIds);

  return (
    <div className="container mx-auto p-6">
      <Link
        href={`/evals/${evalId}`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        {ui("Back to")} {ev?.name ?? ui("Eval")}
      </Link>

      <h1 className="text-2xl font-bold mb-6">
        {ui("Compare runs (")}
        {runIds.length})
      </h1>

      <Card>
        <CardHeader>
          <CardTitle>{ui("Per-case comparison")}</CardTitle>
        </CardHeader>
        <CardContent>
          {ignoredCount > 0 ? (
            <p className="mb-4 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
              {ui("Ignored")} {ignoredCount} {ui("invalid, duplicate, or excess run")}{" "}
              {ignoredCount === 1 ? ui("ID") : ui("IDs")}.
              {exceededLimit
                ? ui(" Compare up to {value0} runs at a time.", { value0: MAX_COMPARE_RUNS })
                : null}
            </p>
          ) : null}
          {runIds.length < 2 ? (
            <p className="text-center py-8 text-muted-foreground">
              {ui("Select at least two runs from the eval’s Runs tab to compare.")}
            </p>
          ) : isLoading ? (
            <Skeleton className="h-64 w-full" />
          ) : isError ? (
            <p className="text-center py-8 text-destructive">
              {ui("Some runs could not be loaded. Check the selected runs and try again.")}
            </p>
          ) : runs.length === 0 ? (
            <p className="text-center py-8 text-muted-foreground">{ui("No results to compare.")}</p>
          ) : (
            <RunComparison evalId={evalId} runs={runs} />
          )}
        </CardContent>
      </Card>
    </div>
  );
}

export default function EvalComparePage({ params }: { params: Promise<{ evalId: string }> }) {
  const { evalId } = use(params);
  return (
    <Suspense
      fallback={
        <div className="container mx-auto p-6">
          <Skeleton className="h-8 w-1/3 mb-4" />
          <Skeleton className="h-64 w-full" />
        </div>
      }
    >
      <CompareContent evalId={evalId} />
    </Suspense>
  );
}
