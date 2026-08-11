"use client";

import { useState } from "react";
import Link from "next/link";
import {
  useHealthCheckRun,
  useLatestHealthCheckRun,
  useTriggerHealthCheck,
} from "@/hooks/use-agents";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import type { HealthCheckCaseResult, HealthCheckRun } from "@/lib/api/types";
import {
  Activity,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  ExternalLink,
  Loader2,
} from "lucide-react";

interface AgentHealthCheckProps {
  agentId: string;
}

/**
 * Behavioral health check: runs generated smoke tests against the agent's real
 * configuration and shows a score card. See knowledge/evaluation/agent-checks.md (tier 3).
 */
export function AgentHealthCheck({ agentId }: AgentHealthCheckProps) {
  const trigger = useTriggerHealthCheck();
  const [runId, setRunId] = useState<string | null>(null);
  const { data: latest } = useLatestHealthCheckRun(agentId);
  const latestRun = latest?.run;
  const latestActiveRunId =
    latestRun?.status === "pending" || latestRun?.status === "running" ? latestRun.id : null;
  const activeRunId = runId ?? latestActiveRunId;
  const { data: triggeredRun } = useHealthCheckRun(agentId, activeRunId);

  // Show a freshly triggered run if there is one; otherwise fall back to the
  // latest persisted run loaded on mount so prior results appear immediately
  // without triggering a new LLM run (EVE-588).
  const run = triggeredRun ?? (runId ? trigger.data : latestRun);
  // Only hint staleness for the mounted latest run, not one we just triggered.
  const configChanged = !runId && !!latest?.run && latest.config_changed;

  // Consider the displayed run (triggered or the latest loaded on mount): a run
  // already in progress keeps the button disabled so we don't start a duplicate.
  const isRunning = trigger.isPending || run?.status === "pending" || run?.status === "running";

  return (
    <Card className="min-w-0 overflow-hidden">
      <CardHeader className="gap-2 p-4">
        <div className="flex flex-wrap items-center gap-2">
          <CardTitle className="flex min-w-0 items-center gap-2 text-base">
            <Activity className="size-4 shrink-0" />
            Health check
          </CardTitle>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="ml-auto max-w-full"
            disabled={isRunning}
            onClick={() =>
              trigger.mutate(agentId, {
                onSuccess: (r) => setRunId(r.id),
              })
            }
          >
            {isRunning ? (
              <>
                <Loader2 className="size-4 animate-spin" />
                Running…
              </>
            ) : (
              "Run health check"
            )}
          </Button>
        </div>
        <CardDescription className="text-xs leading-relaxed">
          Runs generated smoke tests as real sessions, then scores them with an AI judge. Takes a
          minute or two.
        </CardDescription>
      </CardHeader>
      <CardContent className="min-w-0 px-4 pb-4">
        {trigger.error && (
          <p className="text-sm text-destructive">
            Could not start health check: {trigger.error.message}
          </p>
        )}
        {!run && !trigger.error && (
          <p className="text-sm text-muted-foreground italic">
            No health check has been run for this configuration yet.
          </p>
        )}
        {configChanged && (
          <p className="text-sm text-amber-600 dark:text-amber-400 flex items-center gap-2 mb-3">
            <AlertTriangle className="w-4 h-4 shrink-0" />
            This agent&apos;s configuration changed since this run — re-run to refresh results.
          </p>
        )}
        {run && <HealthCheckResults run={run} />}
      </CardContent>
    </Card>
  );
}

function HealthCheckResults({ run }: { run: HealthCheckRun }) {
  if (run.status === "failed") {
    return (
      <p className="text-sm text-destructive">
        Health check failed: {run.error_message ?? "unknown error"}
      </p>
    );
  }
  if (run.status === "pending" || run.status === "running") {
    return (
      <p className="text-sm text-muted-foreground flex items-center gap-2">
        <Loader2 className="w-4 h-4 animate-spin" />
        Generating and running cases…
      </p>
    );
  }

  const summary = run.summary;
  return (
    <div className="min-w-0 space-y-3">
      {summary && (
        <div className="grid grid-cols-2 gap-2">
          <Metric label="Pass rate" value={`${Math.round(summary.pass_rate * 100)}%`} />
          <Metric label="Passed" value={`${summary.passed}/${summary.total}`} />
          <Metric label="Avg score" value={summary.avg_score.toFixed(2)} />
          <Metric label="Avg turns" value={summary.avg_turns.toFixed(1)} />
        </div>
      )}
      {summary && (summary.total_input_tokens > 0 || summary.total_output_tokens > 0) && (
        <p className="text-xs text-muted-foreground">
          {summary.total_input_tokens.toLocaleString()} input /{" "}
          {summary.total_output_tokens.toLocaleString()} output tokens
        </p>
      )}
      {(run.results?.length ?? 0) > 0 && (
        <details className="min-w-0 border p-3">
          <summary className="cursor-pointer text-sm font-medium">
            Case results ({run.results?.length ?? 0})
          </summary>
          <ul className="mt-3 min-w-0 space-y-2">
            {(run.results ?? []).map((result, index) => (
              <CaseRow key={result.session_id ?? `${result.name}-${index}`} result={result} />
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 border bg-muted/30 p-2.5">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="break-words text-base font-semibold">{value}</div>
    </div>
  );
}

function CaseRow({ result }: { result: HealthCheckCaseResult }) {
  const icon = result.error ? (
    <AlertTriangle className="w-4 h-4 text-amber-500 shrink-0 mt-0.5" />
  ) : result.passed ? (
    <CheckCircle2 className="w-4 h-4 text-green-600 shrink-0 mt-0.5" />
  ) : (
    <XCircle className="w-4 h-4 text-destructive shrink-0 mt-0.5" />
  );

  return (
    <li className="flex min-w-0 items-start gap-2 border p-2.5">
      {icon}
      <div className="space-y-1 min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="min-w-0 break-words text-sm font-medium">{result.name}</span>
          {!result.error && (
            <Badge variant="secondary" className="text-xs">
              {result.score.toFixed(2)}
            </Badge>
          )}
          {result.session_id && (
            <Link
              href={`/sessions/${result.session_id}/transcript`}
              target="_blank"
              className="text-muted-foreground hover:text-foreground"
              aria-label="Open session"
            >
              <ExternalLink className="w-3.5 h-3.5" />
            </Link>
          )}
        </div>
        <p className="break-words text-xs text-muted-foreground">{result.user_message}</p>
        {result.error ? (
          <p className="break-words text-xs text-amber-600 dark:text-amber-400">{result.error}</p>
        ) : (
          <p className="break-words text-xs">{result.judge_reason}</p>
        )}
      </div>
    </li>
  );
}
