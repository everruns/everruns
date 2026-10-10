"use client";

/**
 * Inline approval cards for tool calls an agent's `tool_approval` gate held
 * back (EVE-1140).
 *
 * The gate defers a risky call it has no decision for, the backend parks the
 * turn and emits one synthetic `approve_tool_call` call per held-back call,
 * all in one batch. The turn resumes once, so the batch is answered together:
 * with one request a click submits straight away; with several, each card
 * records a choice and the batch is sent when the last one is made. A request
 * the server never hears about is not approved.
 */

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Check, ShieldAlert, X } from "lucide-react";
import { submitToolApprovals, type ToolApprovalDecision } from "@/lib/api/sessions";
import { cn } from "@/lib/utils";
import type { ToolCompletedData } from "@/lib/api/types";

export const TOOL_APPROVAL_TOOL = "approve_tool_call";

/** The request as the backend emitted it. Only display fields are read here. */
export interface ToolApprovalArguments {
  tool?: string;
  display_name?: string;
  arguments?: unknown;
  arguments_truncated?: boolean;
  risk?: string;
  expires_at?: string;
}

export interface ToolApprovalRequest {
  id: string;
  arguments?: unknown;
}

interface ToolApprovalRequestsProps {
  sessionId: string;
  requests: ToolApprovalRequest[];
  /** Existing tool results map — a request with a result shows its outcome. */
  toolResultsMap: Map<string, ToolCompletedData>;
}

const CHOICES: Array<{ decision: ToolApprovalDecision; label: string }> = [
  { decision: "reject_always", label: "Always reject" },
  { decision: "reject", label: "Reject" },
  { decision: "allow_always", label: "Always allow" },
  { decision: "allow", label: "Allow once" },
];

const OUTCOME_LABELS: Record<string, string> = {
  allow: "Allowed once",
  allow_always: "Allowed for this session",
  reject: "Rejected",
  reject_always: "Rejected for this session",
  not_approved: "Not approved",
  expired: "Not approved in time",
};

function riskLine(risk: string | undefined): string {
  switch (risk) {
    case "destructive":
      return "This tool says it can delete or overwrite things.";
    case "open_world":
      return "This tool reaches outside this session.";
    case "rated_changes":
      return "This tool says nothing about its risk, and it looks like it changes things.";
    case "policy":
      return "This call matches the agent's approval policy.";
    default:
      return "This tool changes state.";
  }
}

function prettyArguments(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/** The outcome the server recorded, read from the request's tool result. */
function recordedOutcome(result: ToolCompletedData): string | undefined {
  for (const part of result.result ?? []) {
    if ("text" in part && typeof part.text === "string") {
      const match = /"outcome"\s*:\s*"([a-z_]+)"/.exec(part.text);
      if (match) return match[1];
    }
  }
  return undefined;
}

function isAllowed(outcome: string | undefined): boolean {
  return outcome === "allow" || outcome === "allow_always";
}

function argsOf(request: ToolApprovalRequest): ToolApprovalArguments {
  return (request.arguments ?? {}) as ToolApprovalArguments;
}

export function ToolApprovalRequests({
  sessionId,
  requests,
  toolResultsMap,
}: ToolApprovalRequestsProps) {
  const [choices, setChoices] = useState<Record<string, ToolApprovalDecision>>({});
  const [submitted, setSubmitted] = useState<Record<string, string>>({});
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (requests.length === 0) return null;

  const outcomeOf = (id: string): string | undefined => {
    const existing = toolResultsMap.get(id);
    if (existing) return recordedOutcome(existing) ?? "not_approved";
    return submitted[id];
  };
  const answered = requests.every((request) => outcomeOf(request.id) != null);

  const choose = async (id: string, decision: ToolApprovalDecision) => {
    const next = { ...choices, [id]: decision };
    setChoices(next);
    if (!requests.every((request) => next[request.id] != null)) return;

    setSubmitting(true);
    setError(null);
    try {
      const response = await submitToolApprovals(
        sessionId,
        requests.map((request) => ({ tool_call_id: request.id, decision: next[request.id] })),
      );
      setSubmitted(
        Object.fromEntries(
          response.resolved.map((resolution) => [resolution.tool_call_id, resolution.outcome]),
        ),
      );
    } catch {
      setChoices({});
      setError("Could not record your answer. Try again.");
    } finally {
      setSubmitting(false);
    }
  };

  if (answered) {
    return (
      <div className="space-y-0.5">
        {requests.map((request) => {
          const outcome = outcomeOf(request.id);
          const name = argsOf(request).display_name || argsOf(request).tool || "a tool";
          return (
            <div
              key={request.id}
              className={cn(
                "flex items-center gap-2 px-3 py-1.5 text-sm",
                isAllowed(outcome) ? "text-success" : "text-muted-foreground",
              )}
            >
              {isAllowed(outcome) ? <Check className="h-4 w-4" /> : <X className="h-4 w-4" />}
              {`${OUTCOME_LABELS[outcome ?? ""] ?? "Answered"}: ${name}`}
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {requests.map((request) => {
        const args = argsOf(request);
        const name = args.display_name || args.tool || "a tool";
        const preview = prettyArguments(args.arguments);
        const chosen = choices[request.id];
        const expires = args.expires_at ? new Date(args.expires_at) : null;
        return (
          <div key={request.id} className="border border-border bg-muted/50 px-4 py-3">
            <div className="flex items-start gap-3">
              <ShieldAlert className="mt-0.5 h-5 w-5 shrink-0 text-foreground" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium text-foreground">
                  Allow the agent to run {name}?
                </p>
                <p className="mt-0.5 text-xs text-muted-foreground">
                  {riskLine(args.risk)} It will not run unless you allow it.
                </p>
                {preview && (
                  <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-all font-mono text-xs text-muted-foreground">
                    {preview}
                  </pre>
                )}
                {args.arguments_truncated && (
                  <p className="mt-1 text-xs text-warning">
                    The arguments are longer than shown. Approving covers the full call.
                  </p>
                )}
                {expires && !Number.isNaN(expires.getTime()) && (
                  <p className="mt-1 text-xs text-muted-foreground">
                    Counts as rejected if not answered by {expires.toLocaleTimeString()}.
                  </p>
                )}
                {chosen && requests.length > 1 && !submitting && (
                  <p className="mt-1 text-xs text-muted-foreground">
                    {CHOICES.find((choice) => choice.decision === chosen)?.label} — waiting for your
                    other answers.
                  </p>
                )}
              </div>
            </div>
            <div className="mt-3 flex flex-wrap items-center justify-end gap-2">
              {CHOICES.map((choice) => (
                <Button
                  key={choice.decision}
                  size="sm"
                  variant={
                    choice.decision === "allow"
                      ? "default"
                      : choice.decision === chosen
                        ? "secondary"
                        : "ghost"
                  }
                  onClick={() => void choose(request.id, choice.decision)}
                  disabled={submitting}
                  className={choice.decision === "allow" ? undefined : "text-muted-foreground"}
                >
                  {choice.decision === "allow" && <Check className="mr-1 h-3.5 w-3.5" />}
                  {choice.label}
                </Button>
              ))}
            </div>
          </div>
        );
      })}
      {error && <p className="px-1 text-xs text-destructive">{error}</p>}
    </div>
  );
}
