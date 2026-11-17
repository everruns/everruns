"use client";

/**
 * What the auto reviewer decided about one tool call, beside the call in chat
 * (`tool.reviewed`, knowledge/execution/tool-approval.md "Auto mode").
 *
 * Decisions:
 * - An allowed call gets a quiet badge; the reviewer's reason is its tooltip,
 *   because routine approvals should not crowd the transcript.
 * - A block shows its rule and reason in full: the call did not run, and the
 *   person needs to know why without hovering.
 * - A call handed to a person shows nothing here; the approval card carries
 *   the reviewer's note.
 * - Reads reviews from their own small context, which the session provider
 *   fills. Rows rendered outside a session (dev previews, tests that mock the
 *   session module) get no provider and simply show nothing.
 */

import { createContext, useContext } from "react";
import { Ban, ShieldCheck } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import type { ToolReviewedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";

/** Auto reviewer verdicts by tool_call_id, provided by the session. */
export const ToolReviewsContext = createContext<Map<string, ToolReviewedData> | undefined>(
  undefined,
);

export function useToolReviewsMap(): Map<string, ToolReviewedData> | undefined {
  return useContext(ToolReviewsContext);
}

export function useToolReview(toolCallId: string): ToolReviewedData | undefined {
  return useToolReviewsMap()?.get(toolCallId);
}

export function ToolReviewBadge({
  review,
  className,
}: {
  review: ToolReviewedData | undefined;
  className?: string;
}) {
  const ui = useUiTranslations();
  if (!review) return null;
  if (review.verdict === "allowed") {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center gap-1 text-[11px] font-medium text-success",
          className,
        )}
        title={review.reason}
        data-testid="tool-review-allowed"
      >
        <ShieldCheck className="h-3 w-3" />
        {ui("Auto-allowed")}
      </span>
    );
  }
  if (review.verdict === "blocked") {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center gap-1 text-[11px] font-medium text-destructive",
          className,
        )}
        data-testid="tool-review-blocked"
      >
        <Ban className="h-3 w-3" />
        {ui("Blocked")}
      </span>
    );
  }
  return null;
}

/** The rule and reason of a block, under the call. */
export function ToolReviewBlockReason({
  review,
  className,
}: {
  review: ToolReviewedData | undefined;
  className?: string;
}) {
  if (review?.verdict !== "blocked") return null;
  return (
    <div
      className={cn("mt-1 border-l-2 border-destructive/60 pl-2 text-xs", className)}
      data-testid="tool-review-block-reason"
    >
      {review.rule && <span className="font-medium text-destructive">{review.rule}: </span>}
      <span className="text-foreground/80">{review.reason}</span>
    </div>
  );
}
