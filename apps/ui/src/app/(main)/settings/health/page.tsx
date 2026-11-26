"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageRows } from "@/components/chat-mode/page-view-state";

import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { useState, useEffect } from "react";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { usePageTitle } from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { useHealthIssues, useHealthIssue } from "@/hooks/use-health-issues";
import { useOptionalNotificationsContext } from "@/providers/notifications-provider";
import { HealthIssueDetails } from "@/components/health/health-issue-details";
import { HealthFixQueue } from "@/components/health/health-fix-queue";
import { Button } from "@/components/ui/button";

export default function HealthPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Health"), ui("Settings"));
  const search = useSearchParams();
  const [offset, setOffset] = useState(() => {
    const requested = Number(search.get("offset"));
    return Number.isFinite(requested)
      ? Math.floor(Math.max(0, Math.min(1_000_000, requested)) / 20) * 20
      : 0;
  });
  useChatPageFilters({ offset: offset });
  const policies = usePolicies("agents");
  const canView = policies.can("agent.view");
  const issues = useHealthIssues(undefined, offset, canView);
  useChatPageRows(
    "Health issues",
    (issues.data?.data ?? []).map((i) => ({
      id: i.id,
      title: i.title,
      status: i.status,
      stale: i.stale,
      agent_name: i.agent_name,
    })),
    { enabled: canView, loading: issues.isLoading, error: !!issues.error },
  );

  const id = search.get("issue");
  // Set by the Slack callback when a fixer's approval returns here; the
  // callback has already checked the app again.
  const slackInstall = search.get("slack_install");
  const selected = useHealthIssue(canView ? id : null);
  const notifications = useOptionalNotificationsContext();
  const markViewed = notifications?.markViewed;
  const notificationId = selected.data?.notification_id;
  useEffect(() => {
    if (notificationId && markViewed) void markViewed(notificationId);
  }, [notificationId, markViewed]);
  const pageSize = issues.data?.limit ?? 20;
  const listHref = offset ? `/settings/health?offset=${offset}` : "/settings/health";
  useEffect(() => {
    if (issues.data && offset >= issues.data.total && offset > 0) {
      setOffset(Math.max(0, Math.ceil(issues.data.total / pageSize) - 1) * pageSize);
    }
  }, [issues.data, offset, pageSize]);
  return (
    <div className="@container/health space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">{ui("Health")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui("Review alerts, understand their impact, and choose what to do next.")}
        </p>
      </div>
      {policies.isLoading ? (
        <p>{ui("Loading permissions…")}</p>
      ) : !canView ? (
        <p>{ui("Ask an administrator to review integration health.")}</p>
      ) : (
        <>
          {slackInstall === "ok" && (
            <p role="status" className="text-sm text-success">
              {ui("Slack approved the change. Everruns checked the app again.")}
            </p>
          )}
          {slackInstall === "failed" && (
            <p role="alert" className="text-sm text-destructive">
              {ui("Slack did not finish the approval. Try the fix again.")}
            </p>
          )}
          {policies.can("agent.manage") && issues.data && (
            <HealthFixQueue issues={issues.data.data} />
          )}
          <div className="grid items-start gap-6 @[800px]/health:grid-cols-[minmax(240px,320px)_minmax(0,1fr)]">
            <nav
              aria-label={ui("Health issues")}
              className={id ? "hidden @[800px]/health:block" : ""}
            >
              <h3 className="mb-3 text-sm font-medium">
                {ui("Open alerts · {value0}", { value0: issues.data?.total ?? 0 })}
              </h3>
              {issues.isLoading && (
                <p className="text-sm text-muted-foreground">{ui("Loading health issues…")}</p>
              )}
              {issues.isError && (
                <p role="alert" className="text-sm text-destructive">
                  {ui("Could not load health issues.")}{" "}
                  <Button size="sm" variant="ghost" onClick={() => issues.refetch()}>
                    {ui("Retry")}
                  </Button>
                </p>
              )}
              {issues.data?.total === 0 && (
                <p className="text-sm text-muted-foreground">
                  {ui(
                    "No pending issues detected. Checks reflect the latest available observations.",
                  )}
                </p>
              )}
              <ul className="space-y-2">
                {(issues.data?.data ?? []).map((issue) => (
                  <li key={issue.id}>
                    <Link
                      href={`${listHref}${offset ? "&" : "?"}issue=${encodeURIComponent(issue.id)}`}
                      aria-current={issue.id === id ? "true" : undefined}
                      className={`block space-y-1 border p-3 hover:bg-muted ${issue.id === id ? "border-primary bg-muted" : "border-border"}`}
                    >
                      <p className="text-sm font-medium">{issue.title}</p>
                      <p className="text-xs text-muted-foreground">
                        {issue.agent_name ?? ui("Organization")}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        {issue.status === "needs_check"
                          ? ui("Needs check")
                          : issue.stale
                            ? ui("Verification stale")
                            : issue.code === "agent.unattended_spend_spike"
                              ? ui("Warning")
                              : ui("Action required")}
                      </p>
                    </Link>
                  </li>
                ))}
              </ul>
              {issues.data && (issues.data.total > pageSize || offset > 0) && (
                <div className="mt-4 space-y-2">
                  <p className="text-xs text-muted-foreground">
                    {ui("{from}–{to} of {total}", {
                      from: offset + 1,
                      to: Math.min(offset + pageSize, issues.data.total),
                      total: issues.data.total,
                    })}
                  </p>
                  <div className="flex gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={offset === 0}
                      onClick={() => setOffset(Math.max(0, offset - pageSize))}
                    >
                      {ui("Previous")}
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={offset + pageSize >= issues.data.total}
                      onClick={() => setOffset(offset + pageSize)}
                    >
                      {ui("Next")}
                    </Button>
                  </div>
                </div>
              )}
            </nav>
            <div className={`min-w-0 space-y-4 ${id ? "" : "hidden @[800px]/health:block"}`}>
              {id && (
                <Link
                  className="inline-block text-sm underline underline-offset-4 @[800px]/health:hidden"
                  href={listHref}
                >
                  {ui("Back to issues")}
                </Link>
              )}
              {id && selected.isLoading && <p role="status">{ui("Loading health issues…")}</p>}
              {selected.isError && (
                <p role="alert" className="text-sm text-destructive">
                  {ui("Could not load this health issue. It may no longer be available.")}
                </p>
              )}
              {selected.data && (
                <HealthIssueDetails
                  key={selected.data.id}
                  issue={selected.data}
                  canManage={policies.can("agent.manage")}
                />
              )}
              {!id && (
                <p className="border border-dashed p-6 text-sm text-muted-foreground">
                  {ui("Select an issue to see its details and actions.")}
                </p>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
