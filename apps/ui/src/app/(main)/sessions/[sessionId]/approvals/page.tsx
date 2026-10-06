"use client";

// Approvals — what this session asked for, and what was recorded as approved.
// A request and its grant are one card. The org-wide list that showed them as
// unrelated rows is gone; an admin reads the pair on the session it belongs to.

import { SessionApprovals } from "@/components/session/session-approvals";
import { useSessionApprovals } from "@/hooks/use-session-approvals";
import { useSessionContext } from "../session-context";

export default function SessionApprovalsPage() {
  const { sessionId, events, eventsLoading, hasMoreEvents } = useSessionContext();
  const { episodes, loading, error, retry } = useSessionApprovals(
    sessionId,
    events,
    eventsLoading,
    hasMoreEvents,
  );

  return (
    <section aria-label="Approvals" className="min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-6">
      <SessionApprovals
        sessionId={sessionId}
        episodes={episodes}
        loading={loading}
        error={error}
        onRetry={retry}
      />
    </section>
  );
}
