"use client";

// Approvals — what this session asked for, and what was recorded as approved.
// A request and its grant are one card. The org-wide list that showed them as
// unrelated rows is gone; an admin reads the pair on the session it belongs to.

import { useUiTranslations } from "@/providers/locale-provider";
import { getEventData } from "@/lib/api/types";
import {
  TOOL_APPROVAL_TOOL,
  ToolApprovalRequests,
} from "@/components/chat/tool-approval-tool-call";
import { SessionApprovals } from "@/components/session/session-approvals";
import { useSessionApprovals } from "@/hooks/use-session-approvals";
import { useSessionContext } from "../session-context";

export default function SessionApprovalsPage() {
  const ui = useUiTranslations();
  const { sessionId, events, eventsLoading, hasMoreEvents, toolResultsMap } = useSessionContext();
  // Follow the latest parked batch, rather than a locally projected running
  // status: session.activated remains the last status event while tools wait.
  const latestBoundary = events?.findLast((event) =>
    [
      "tool.call_requested",
      "session.activated",
      "session.idled",
      "turn.failed",
      "turn.cancelled",
    ].includes(event.type),
  );
  const requests = latestBoundary
    ? (getEventData(latestBoundary, "tool.call_requested")?.tool_calls ?? []).filter(
        (call) =>
          call.name === TOOL_APPROVAL_TOOL &&
          call.id.startsWith("tool_approval_") &&
          !toolResultsMap.has(call.id),
      )
    : [];
  const { episodes, loading, error, retry } = useSessionApprovals(
    sessionId,
    events,
    eventsLoading,
    hasMoreEvents,
  );

  return (
    <section
      aria-label={ui("Approvals")}
      className="min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-6"
    >
      {requests.length > 0 && (
        <div className="mb-6 max-w-3xl">
          <h2 className="mb-3 text-sm font-medium">{ui("Tool approvals")}</h2>
          <ToolApprovalRequests
            key={latestBoundary?.id}
            sessionId={sessionId}
            requests={requests}
            toolResultsMap={toolResultsMap}
          />
        </div>
      )}
      {(requests.length === 0 || episodes.length > 0 || loading || error) && (
        <SessionApprovals
          sessionId={sessionId}
          episodes={episodes}
          loading={loading}
          error={error}
          onRetry={retry}
        />
      )}
    </section>
  );
}
