"use client";

import { use, Suspense } from "react";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { useEval, useEvalRunComparison, usePageTitle } from "@/hooks";
import { RunComparison } from "@/components/evals/run-comparison";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft } from "lucide-react";

function CompareContent({ evalId }: { evalId: string }) {
  const searchParams = useSearchParams();
  const runIds = (searchParams.get("runs") ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

  const { data: ev } = useEval(evalId);
  usePageTitle("Compare runs", ev?.name ?? null, "Eval");
  const { runs, isLoading } = useEvalRunComparison(evalId, runIds);

  return (
    <div className="container mx-auto p-6">
      <Link
        href={`/evals/${evalId}`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to {ev?.name ?? "Eval"}
      </Link>

      <h1 className="text-2xl font-bold mb-6">Compare runs ({runIds.length})</h1>

      <Card>
        <CardHeader>
          <CardTitle>Per-case comparison</CardTitle>
        </CardHeader>
        <CardContent>
          {runIds.length < 2 ? (
            <p className="text-center py-8 text-muted-foreground">
              Select at least two runs from the eval&rsquo;s Runs tab to compare.
            </p>
          ) : isLoading ? (
            <Skeleton className="h-64 w-full" />
          ) : runs.length === 0 ? (
            <p className="text-center py-8 text-muted-foreground">No results to compare.</p>
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
