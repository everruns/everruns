/**
 * Decisions:
 * - The runtime decides severity, not the UI. `tool.completed` carries
 *   `severity` on failure: a call that failed and went back to the model is an
 *   "issue", and the turn carried on. Only work that stopped (a failed turn,
 *   session, or task) is an "error" and earns destructive red.
 * - Failed calls recorded before `severity` existed read as issues, which is
 *   what every one of them was.
 */
import { AlertCircle, AlertTriangle } from "lucide-react";
import type { FailureSeverity, ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";

export type { FailureSeverity };

/** Severity of a finished tool call, or null when it did not fail. */
export function toolFailureSeverity(
  result?: Pick<ToolCompletedData, "success" | "error" | "severity"> | null,
): FailureSeverity | null {
  if (!result) return null;
  if (result.success && !result.error) return null;
  return result.severity ?? "issue";
}

/** Text color for a failure marker or its message. */
export function failureTextClass(severity: FailureSeverity): string {
  return severity === "error" ? "text-destructive" : "text-warning";
}

/** Small marker icon: a warning triangle for issues, a red circle for errors. */
export function FailureIcon({
  severity,
  className,
}: {
  severity: FailureSeverity;
  className?: string;
}) {
  const Icon = severity === "error" ? AlertCircle : AlertTriangle;
  return (
    <Icon
      aria-hidden="true"
      data-severity={severity}
      className={cn("h-3.5 w-3.5", failureTextClass(severity), className)}
    />
  );
}

/** Recoverable and fatal failure tallies for a stretch of work. */
export interface FailureCounts {
  issues: number;
  errors: number;
}

export const NO_FAILURES: FailureCounts = { issues: 0, errors: 0 };

/** Tally failed calls among `toolCalls` by severity. */
export function countToolCallFailures(
  toolCalls: ReadonlyArray<{ id: string }>,
  results: ReadonlyMap<string, ToolCompletedData>,
): FailureCounts {
  const counts = { issues: 0, errors: 0 };
  for (const toolCall of toolCalls) {
    const severity = toolFailureSeverity(results.get(toolCall.id));
    if (severity === "issue") counts.issues += 1;
    else if (severity === "error") counts.errors += 1;
  }
  return counts;
}

/** Sum tallies into the work log header's `issueCount`/`errorCount` props. */
export function sumFailureCounts(...counts: FailureCounts[]): {
  issueCount: number;
  errorCount: number;
} {
  return counts.reduce(
    (total, next) => ({
      issueCount: total.issueCount + next.issues,
      errorCount: total.errorCount + next.errors,
    }),
    { issueCount: 0, errorCount: 0 },
  );
}
