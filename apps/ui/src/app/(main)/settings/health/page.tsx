"use client";

import { useState, useEffect } from "react";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { usePageTitle } from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { useHealthIssues, useHealthIssue } from "@/hooks/use-health-issues";
import { useOptionalNotificationsContext } from "@/providers/notifications-provider";
import { HealthIssueDetails } from "@/components/health/health-issue-details";
import { Button } from "@/components/ui/button";

export default function HealthPage() {
  usePageTitle("Health", "Settings");
  const [offset, setOffset] = useState(0);
  const policies = usePolicies("agents");
  const canView = policies.can("agent.view");
  const issues = useHealthIssues(undefined, offset, canView);
  const id = useSearchParams().get("issue");
  const selected = useHealthIssue(canView ? id : null);
  const notifications = useOptionalNotificationsContext();
  const markViewed = notifications?.markViewed;
  const notificationId = selected.data?.notification_id;
  useEffect(() => {
    if (notificationId && markViewed) void markViewed(notificationId);
  }, [notificationId, markViewed]);
  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">Health</h2>
        <p className="text-sm text-muted-foreground">
          Pending organization and integration issues, and the actions that restore them. Reading or
          snoozing a notification does not resolve an issue.
        </p>
      </div>
      {policies.isLoading ? (
        <p>Loading permissions…</p>
      ) : !canView ? (
        <p>Ask an administrator to review integration health.</p>
      ) : (
        <>
          {selected.isError && (
            <p role="alert" className="text-sm text-destructive">
              Could not load this health issue. It may no longer be available.
            </p>
          )}
          {selected.data && (
            <HealthIssueDetails
              key={selected.data.id}
              issue={selected.data}
              canManage={policies.can("agent.manage")}
            />
          )}
          {issues.isLoading && (
            <p className="text-sm text-muted-foreground">Loading health issues…</p>
          )}
          {issues.isError && (
            <p role="alert" className="text-sm text-destructive">
              Could not load health issues.{" "}
              <Button size="sm" variant="ghost" onClick={() => issues.refetch()}>
                Retry
              </Button>
            </p>
          )}
          {issues.data && (
            <>
              <h3 className="text-sm font-medium">
                Action required · {issues.data.total} unresolved
              </h3>
              {issues.data.total === 0 && (
                <p className="text-sm text-muted-foreground">
                  No pending issues detected. Checks reflect the latest available observations.
                </p>
              )}
              <div className="divide-y divide-border">
                {issues.data.data.map((issue) => (
                  <Link
                    href={issue.href}
                    key={issue.id}
                    className="block space-y-1 py-4 hover:bg-muted"
                  >
                    <p className="text-sm font-medium">{issue.title}</p>
                    <p className="text-xs text-muted-foreground">
                      {issue.agent_name ?? "Organization"} ·{" "}
                      {issue.status === "needs_check"
                        ? "Needs check"
                        : issue.stale
                          ? "Verification stale"
                          : "Action required"}
                    </p>
                    <p className="text-sm">{issue.body}</p>
                  </Link>
                ))}
              </div>
              <div className="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - 20))}
                >
                  Previous
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={offset + 20 >= issues.data.total}
                  onClick={() => setOffset(offset + 20)}
                >
                  Next
                </Button>
              </div>
            </>
          )}
        </>
      )}
    </div>
  );
}
