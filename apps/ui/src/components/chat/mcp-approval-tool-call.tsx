"use client";

/**
 * Inline approval card for a remote MCP call OpenAI wants to make (EVE-1115).
 *
 * With the OpenAI Server Tools capability, OpenAI calls remote MCP servers
 * itself. Before each call it stops and asks; the backend pauses the turn and
 * emits a synthetic `openai_mcp_approval` call carrying the server, the tool
 * and the exact arguments. This card is where a person decides.
 *
 * The answer goes back as the call's tool result. Only `{ approve: true }`
 * approves, so anything unexpected, an error included, is a denial.
 */

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Check, ShieldQuestion, X } from "lucide-react";
import { submitToolResults } from "@/lib/api/sessions";
import { cn } from "@/lib/utils";
import type { ToolCompletedData } from "@/lib/api/types";

export const MCP_APPROVAL_TOOL = "openai_mcp_approval";

export interface McpApprovalArguments {
  server_label?: string;
  name?: string;
  /** The MCP tool's arguments as the JSON string OpenAI sent. */
  arguments?: string;
}

interface McpApprovalToolCallProps {
  sessionId: string;
  toolCallId: string;
  approval: McpApprovalArguments;
  /** Existing tool results map — if a result already exists, show completed state */
  toolResultsMap: Map<string, ToolCompletedData>;
}

function prettyArguments(raw: string | undefined): string {
  if (!raw) return "";
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

function wasApproved(result: ToolCompletedData): boolean {
  return (result.result ?? []).some(
    (part) =>
      "text" in part && typeof part.text === "string" && /"approve"\s*:\s*true/.test(part.text),
  );
}

export function McpApprovalToolCall({
  sessionId,
  toolCallId,
  approval,
  toolResultsMap,
}: McpApprovalToolCallProps) {
  const [status, setStatus] = useState<"idle" | "submitting" | "approved" | "denied">("idle");
  const [error, setError] = useState<string | null>(null);

  const server = approval.server_label || "An MCP server";
  const tool = approval.name || "a tool";
  const args = prettyArguments(approval.arguments);

  const existingResult = toolResultsMap.get(toolCallId);
  const isCompleted = existingResult != null || status === "approved" || status === "denied";

  const decide = async (approve: boolean) => {
    setStatus("submitting");
    setError(null);
    try {
      await submitToolResults(sessionId, [{ tool_call_id: toolCallId, result: { approve } }]);
      setStatus(approve ? "approved" : "denied");
    } catch {
      setStatus("idle");
      setError("Could not record your answer. Try again.");
    }
  };

  if (isCompleted) {
    const approved =
      status === "approved" ||
      (status !== "denied" && existingResult != null && wasApproved(existingResult));
    return (
      <div
        className={cn(
          "flex items-center gap-2 px-3 py-1.5 text-sm",
          approved ? "text-success" : "text-muted-foreground",
        )}
      >
        {approved ? <Check className="h-4 w-4" /> : <X className="h-4 w-4" />}
        {approved ? `Approved ${tool} on ${server}` : `Denied ${tool} on ${server}`}
      </div>
    );
  }

  return (
    <div className="border border-border bg-muted/50 px-4 py-3">
      <div className="flex items-start gap-3">
        <ShieldQuestion className="mt-0.5 h-5 w-5 shrink-0 text-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground">
            Allow {server} to run {tool}?
          </p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            OpenAI will call this remote MCP server with these arguments.
          </p>
          {args && (
            <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-all font-mono text-xs text-muted-foreground">
              {args}
            </pre>
          )}
          {error && <p className="mt-2 text-xs text-destructive">{error}</p>}
        </div>
      </div>
      <div className="mt-3 flex items-center justify-end gap-2">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void decide(false)}
          disabled={status === "submitting"}
          className="text-muted-foreground"
        >
          Deny
        </Button>
        <Button size="sm" onClick={() => void decide(true)} disabled={status === "submitting"}>
          <Check className="mr-1 h-3.5 w-3.5" />
          Approve
        </Button>
      </div>
    </div>
  );
}
