"use client";

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { listAuditLogs, type AuditLogEntry } from "@/lib/api/audit-logs";
import { listToolCompletionEvents } from "@/lib/api/events";
import type { Event } from "@/lib/api/types";
import { buildApprovalEpisodes } from "@/lib/approval-episodes";
import { queryKeys } from "@/lib/query-keys";
import { useMembers } from "@/hooks/use-members";
import { useOrg } from "@/providers/org-provider";

const AUDIT_PAGE_SIZE = 100;
const AUDIT_PAGE_CAP = 5;

async function loadApprovalToolEvents(sessionId: string): Promise<Event[]> {
  const [asks, grants] = await Promise.all([
    listToolCompletionEvents(sessionId, "request_approval"),
    listToolCompletionEvents(sessionId, "record_approval"),
  ]);
  return [...asks, ...grants];
}

/** Audit rows for this session. A viewer who cannot read the org audit log gets none. */
async function loadApprovalAudit(orgId: string, sessionId: string): Promise<AuditLogEntry[]> {
  const matched: AuditLogEntry[] = [];
  let before: string | undefined;
  for (let page = 0; page < AUDIT_PAGE_CAP; page += 1) {
    let batch: AuditLogEntry[];
    try {
      batch = await listAuditLogs(orgId, {
        eventType: "agent.approval.",
        limit: AUDIT_PAGE_SIZE,
        before,
      });
    } catch {
      return matched;
    }
    matched.push(
      ...batch.filter((entry) => entry.target_type === "session" && entry.target_id === sessionId),
    );
    if (batch.length < AUDIT_PAGE_SIZE) break;
    const cursor = batch.at(-1)?.created_at;
    if (!cursor || cursor === before) break;
    before = cursor;
  }
  return matched;
}

function mergeEvents(live: readonly Event[], historical: readonly Event[]): Event[] {
  const byId = new Map<string, Event>();
  for (const event of historical) byId.set(event.id, event);
  for (const event of live) byId.set(event.id, event);
  return [...byId.values()];
}

export function useSessionApprovals(
  sessionId: string,
  events: Event[] | undefined,
  eventsLoading: boolean,
  hasMoreEvents: boolean,
) {
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.public_id;

  const history = useQuery({
    queryKey: queryKeys.sessions.approvalEvents(sessionId),
    queryFn: () => loadApprovalToolEvents(sessionId),
  });
  const audit = useQuery({
    queryKey: queryKeys.sessions.approvalAudit(orgId, sessionId),
    queryFn: () => loadApprovalAudit(orgId!, sessionId),
    enabled: Boolean(orgId),
    retry: false,
  });

  const merged = useMemo(
    () => mergeEvents(events ?? [], history.data ?? []),
    [events, history.data],
  );
  const { data: members } = useMembers(merged.length > 0 || (audit.data?.length ?? 0) > 0);
  const memberNames = useMemo(
    () => new Map((members ?? []).map((member) => [member.user_id, member.name || member.email])),
    [members],
  );

  const loadedSinceSequence = useMemo(() => {
    const sequences = (events ?? [])
      .map((event) => event.sequence)
      .filter((sequence): sequence is number => sequence !== undefined);
    return sequences.length > 0 ? Math.min(...sequences) : undefined;
  }, [events]);

  const episodes = useMemo(
    () =>
      buildApprovalEpisodes(merged, {
        memberNames,
        auditEntries: audit.data,
        // Input messages come from the transcript window. Until that window is
        // complete, an ask older than it is not "still waiting" — we cannot see
        // whether someone already replied.
        inputsComplete: !eventsLoading && !hasMoreEvents,
        loadedSinceSequence,
      }),
    [merged, memberNames, audit.data, eventsLoading, hasMoreEvents, loadedSinceSequence],
  );

  return {
    episodes,
    loading: (eventsLoading || history.isLoading) && episodes.length === 0,
    error: history.isError
      ? history.error instanceof Error
        ? history.error.message
        : "The approval history could not be loaded."
      : undefined,
    retry: () => {
      void history.refetch();
    },
  };
}
