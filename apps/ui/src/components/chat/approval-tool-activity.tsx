"use client";

import Link from "next/link";
import { Check, ExternalLink, Loader2, ShieldAlert } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "./tool-call-utils";

export interface ApprovalToolContext {
  approvedBy?: string;
  consentMessageHref?: string;
}

export function ApprovalToolActivity({
  toolCall,
  toolResult,
  context,
}: {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
  context?: ApprovalToolContext;
}) {
  const action = toolCall.arguments.action;
  const detail = toolCall.arguments.detail ?? toolCall.arguments.question;
  const isRequest = toolCall.name === "request_approval";
  const failed = toolResult && (!toolResult.success || Boolean(toolResult.error));
  const complete = Boolean(toolResult?.success);
  const title = failed
    ? "Approval could not be recorded"
    : isRequest
      ? "Approval requested"
      : complete
        ? "Approval recorded"
        : "Recording approval…";

  return (
    <div className="flex items-start gap-3 border border-border/70 bg-card px-3.5 py-3">
      <span className="mt-0.5 inline-flex size-7 shrink-0 items-center justify-center bg-accent/25 text-foreground">
        {isRequest ? (
          <ShieldAlert className="size-4" />
        ) : failed ? (
          <ShieldAlert className="size-4" />
        ) : complete ? (
          <Check className="size-4" />
        ) : (
          <Loader2 className="size-4 animate-spin" />
        )}
      </span>
      <div className="min-w-0 flex-1">
        <div className="font-medium text-foreground">{title}</div>
        {typeof action === "string" && action.trim() && (
          <div className="mt-0.5 text-sm text-foreground">{action}</div>
        )}
        {typeof detail === "string" && detail.trim() && (
          <div className="mt-1 whitespace-pre-wrap text-sm text-muted-foreground">{detail}</div>
        )}
        {!isRequest && complete && (
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
            <span>Approved by {context?.approvedBy ?? "an unknown actor"}</span>
            {context?.consentMessageHref && (
              <Link
                href={context.consentMessageHref}
                className="inline-flex items-center gap-1 text-primary hover:underline"
              >
                View consent <ExternalLink className="size-3" />
              </Link>
            )}
          </div>
        )}
        {failed && toolResult?.error && (
          <div className="mt-1 text-xs text-destructive">{toolResult.error}</div>
        )}
      </div>
    </div>
  );
}
