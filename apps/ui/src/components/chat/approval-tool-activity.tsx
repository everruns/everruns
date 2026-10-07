"use client";

import Link from "next/link";
import { useId, useState } from "react";
import { Check, ChevronRight, ExternalLink, Loader2, ShieldAlert } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
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
  const [isExpanded, setIsExpanded] = useState(false);
  const detailsId = useId();
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
    <div className="min-w-0">
      {/* Consent lives in chat; this row is a quiet receipt with optional audit details. */}
      <button
        type="button"
        aria-expanded={isExpanded}
        aria-controls={detailsId}
        onClick={() => setIsExpanded((current) => !current)}
        className="flex max-w-full items-center gap-1.5 py-1 text-left text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        {failed || isRequest ? (
          <ShieldAlert aria-hidden="true" className="size-3.5 shrink-0" />
        ) : complete ? (
          <Check aria-hidden="true" className="size-3.5 shrink-0" />
        ) : (
          <Loader2 aria-hidden="true" className="size-3.5 shrink-0 animate-spin" />
        )}
        <span>{title}</span>
        <ChevronRight
          aria-hidden="true"
          className={cn("size-3 shrink-0", isExpanded && "rotate-90")}
        />
      </button>
      <div id={detailsId} hidden={!isExpanded}>
        {isExpanded && (
          <div className="space-y-1 pb-1 pl-5 pt-2 text-sm text-muted-foreground [overflow-wrap:anywhere]">
            {typeof action === "string" && action.trim() && (
              <div className="whitespace-pre-wrap text-foreground">{action}</div>
            )}
            {typeof detail === "string" && detail.trim() && (
              <div className="whitespace-pre-wrap">{detail}</div>
            )}
            {!isRequest && complete && (
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
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
          </div>
        )}
      </div>
      {failed && toolResult?.error && (
        <div className="mt-1 pl-5 text-xs text-destructive [overflow-wrap:anywhere]">
          {toolResult.error}
        </div>
      )}
    </div>
  );
}
