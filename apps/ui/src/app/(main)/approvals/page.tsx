"use client";

import { useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { Check, ExternalLink, ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { PageHeader, PageShell } from "@/components/layout/page-shell";
import { listAuditLogs, type AuditLogEntry } from "@/lib/api/audit-logs";
import { useMembers } from "@/hooks/use-members";
import { usePageTitle } from "@/hooks/use-page-title";
import { useOrg } from "@/providers/org-provider";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 100;
type ApprovalFilter = "all" | "granted" | "requested";

function detail(entry: AuditLogEntry, key: string): string | undefined {
  const value = entry.metadata?.[key];
  return typeof value === "string" && value.trim() ? value : undefined;
}

function approvalKind(entry: AuditLogEntry): "granted" | "requested" {
  return entry.event_type === "agent.approval.granted" ? "granted" : "requested";
}

function readableTimestamp(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export default function ApprovalsPage() {
  usePageTitle("Approvals");
  const [filter, setFilter] = useState<ApprovalFilter>("all");
  const { currentOrg, hasRole } = useOrg();
  const canView = hasRole("admin");
  const { data: members } = useMembers(canView);
  const query = useInfiniteQuery({
    queryKey: ["approval-audit", currentOrg?.public_id],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) =>
      listAuditLogs(currentOrg!.public_id, {
        eventType: "agent.approval.",
        limit: PAGE_SIZE,
        before: pageParam,
      }),
    getNextPageParam: (lastPage) =>
      lastPage.length === PAGE_SIZE ? lastPage.at(-1)?.created_at : undefined,
    enabled: canView && !!currentOrg,
  });

  if (!canView) {
    return (
      <PageShell>
        <PageHeader
          title="Approvals"
          description="Approval history is available to organization admins and owners."
        />
      </PageShell>
    );
  }

  const entries = query.data?.pages.flat() ?? [];
  const filteredEntries = entries.filter(
    (entry) => filter === "all" || approvalKind(entry) === filter,
  );
  const memberNames = new Map(
    (members ?? []).map((member) => [member.user_id, member.name || member.email]),
  );

  return (
    <PageShell>
      <PageHeader
        title={
          <span className="flex items-center gap-3">
            <ShieldCheck className="size-6 text-primary" />
            Approvals
          </span>
        }
        description="Review spoken-consent requests and grants across your organization."
      />

      <div className="mb-4 flex flex-wrap gap-2" aria-label="Filter approvals">
        {(
          [
            ["all", "All activity"],
            ["granted", "Recorded"],
            ["requested", "Requested"],
          ] as const
        ).map(([value, label]) => (
          <Button
            key={value}
            variant={filter === value ? "default" : "outline"}
            size="sm"
            onClick={() => setFilter(value)}
            aria-pressed={filter === value}
          >
            {label}
          </Button>
        ))}
        <span className="ml-auto self-center text-xs text-muted-foreground">
          {filteredEntries.length} shown
        </span>
      </div>

      {query.isLoading ? (
        <p className="py-12 text-center text-sm text-muted-foreground">Loading approval history…</p>
      ) : query.isError ? (
        <Card>
          <CardContent className="py-10 text-center">
            <p className="font-medium">Couldn’t load approval history</p>
            <p className="mt-1 text-sm text-muted-foreground">{query.error.message}</p>
            <Button className="mt-4" variant="outline" onClick={() => void query.refetch()}>
              Try again
            </Button>
          </CardContent>
        </Card>
      ) : filteredEntries.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center">
            <ShieldCheck className="mx-auto size-8 text-muted-foreground/60" />
            <p className="mt-3 font-medium">
              {entries.length === 0
                ? "No approval activity yet"
                : "No activity matches this filter"}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {entries.length === 0
                ? "Approval requests and consent will appear here as agents use them."
                : "Choose another filter to see the available approval activity."}
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-3">
          {filteredEntries.map((entry) => {
            const kind = approvalKind(entry);
            const action = detail(entry, "approved_action") ?? "Critical action";
            const extra = detail(entry, kind === "granted" ? "approved_detail" : "question");
            const actor = entry.actor_id ? memberNames.get(entry.actor_id) : undefined;
            const sessionId = entry.target_type === "session" ? entry.target_id : undefined;
            const consentMessageId = detail(entry, "approved_in_message");
            const approvedBy = actor
              ? `Approved by ${actor}`
              : entry.actor_id
                ? `Approved by user ${entry.actor_id}`
                : detail(entry, "actor_resolution") === "unattributed"
                  ? "Approved by an unattributed initiator"
                  : "Approved by an unknown actor";

            return (
              <Card key={entry.id}>
                <CardContent className="flex flex-col gap-3 py-4 sm:flex-row sm:items-center sm:gap-5">
                  <div className="flex min-w-0 flex-1 items-start gap-3">
                    <span
                      className={cn(
                        "mt-0.5 inline-flex size-8 shrink-0 items-center justify-center border",
                        kind === "granted"
                          ? "border-accent bg-accent/20 text-foreground"
                          : "border-border bg-muted text-muted-foreground",
                      )}
                    >
                      {kind === "granted" ? (
                        <Check className="size-4" />
                      ) : (
                        <ShieldCheck className="size-4" />
                      )}
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="font-medium">{action}</p>
                        <Badge variant={kind === "granted" ? "default" : "secondary"}>
                          {kind === "granted" ? "Recorded" : "Requested"}
                        </Badge>
                      </div>
                      {extra && (
                        <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">{extra}</p>
                      )}
                      <p className="mt-1 text-xs text-muted-foreground">
                        {kind === "granted" ? approvedBy : "Approval requested"}
                        <span aria-hidden> · </span>
                        {readableTimestamp(entry.created_at)}
                      </p>
                    </div>
                  </div>
                  {sessionId && (
                    <LinkButton
                      href={`/sessions/${sessionId}/chat${consentMessageId ? `#message-${consentMessageId}` : ""}`}
                      variant="outline"
                      size="sm"
                      className="shrink-0"
                    >
                      {consentMessageId ? "View consent" : "Open session"}
                      <ExternalLink className="ml-2 size-3.5" />
                    </LinkButton>
                  )}
                </CardContent>
              </Card>
            );
          })}
          {query.hasNextPage && (
            <div className="flex justify-center pt-2">
              <Button
                variant="outline"
                onClick={() => void query.fetchNextPage()}
                disabled={query.isFetchingNextPage}
              >
                {query.isFetchingNextPage ? "Loading…" : "Load older approvals"}
              </Button>
            </div>
          )}
        </div>
      )}
    </PageShell>
  );
}
