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
 * configuration and shows a score card. See specs/agent-checks.md (tier 3).
 */
export function AgentHealthCheck({ agentId }: AgentHealthCheckProps) {
  const trigger = useTriggerHealthCheck();
  const [runId, setRunId] = useState<string | null>(null);
  const { data: triggeredRun } = useHealthCheckRun(agentId, runId);
  const { data: latest } = useLatestHealthCheckRun(agentId);

  // Show a freshly triggered run if there is one; otherwise fall back to the
  // latest persisted run loaded on mount so prior results appear immediately
  // without triggering a new LLM run (EVE-588).
  const run = triggeredRun ?? latest?.run;
  // Only hint staleness for the mounted latest run, not one we just triggered.
  const configChanged = !runId && !!latest?.run && latest.config_changed;

  // Consider the displayed run (triggered or the latest loaded on mount): a run
  // already in progress keeps the button disabled so we don't start a duplicate.
  const isRunning = trigger.isPending || run?.status === "pending" || run?.status === "running";

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Activity className="w-5 h-5" />
          Health check
          <span className="flex-1" />
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={isRunning}
            onClick={() =>
              trigger.mutate(agentId, {
                onSuccess: (r) => setRunId(r.id),
              })
            }
          >
            {isRunning ? (
              <>
                <Loader2 className="w-4 h-4 mr-1 animate-spin" />
                Running…
              </>
            ) : (
              "Run health check"
            )}
          </Button>
        </CardTitle>
        <CardDescription>
          Generates a few smoke tests from this agent&apos;s configuration and runs them as real
          sessions, then scores each with an AI judge. Runs several real sessions and takes a minute
          or two.
        </CardDescription>
      </CardHeader>
      <CardContent>
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
    <div className="space-y-4">
      {summary && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
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
      <ul className="space-y-2">
        {(run.results ?? []).map((result, index) => (
          <CaseRow key={result.session_id ?? `${result.name}-${index}`} result={result} />
        ))}
      </ul>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="border p-3 bg-muted/30">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-lg font-semibold">{value}</div>
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
    <li className="flex items-start gap-3 border p-3">
      {icon}
      <div className="space-y-1 min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">{result.name}</span>
          {!result.error && (
            <Badge variant="secondary" className="text-xs">
              {result.score.toFixed(2)}
            </Badge>
          )}
          {result.session_id && (
            <Link
              href={`/sessions/${result.session_id}`}
              target="_blank"
              className="text-muted-foreground hover:text-foreground"
              aria-label="Open session"
            >
              <ExternalLink className="w-3.5 h-3.5" />
            </Link>
          )}
        </div>
        <p className="text-xs text-muted-foreground">{result.user_message}</p>
        {result.error ? (
          <p className="text-xs text-amber-600 dark:text-amber-400">{result.error}</p>
        ) : (
          <p className="text-xs">{result.judge_reason}</p>
        )}
      </div>
    </li>
  );
}
