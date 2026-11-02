"use client";

// One finding (approved mockup 04): what a detector or classifier saw, the
// numbers it recorded, and the Inspector's account of what happened, with
// the cluster of similar findings it filed this one under. Actions: open the
// session, and dismiss with a reason the detectors are calibrated against.
// The suggested-fix card checks and applies the Inspector's fix with a
// person's approval; the mockup's automatic-action card arrives with
// self-healing, and nothing is changed automatically today.

import { use, useState } from "react";
import { useRouter } from "next/navigation";
import { ExternalLink } from "lucide-react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { usePageTitle } from "@/hooks";
import { useDismissFinding, useHealthIssue } from "@/hooks/use-health-issues";
import { usePolicies } from "@/hooks/use-policies";
import { EvalsIcon } from "@/components/icons/facet-icons";
import { ResourceNotFound } from "@/components/resource-not-found";
import { PageBreadcrumb, PageContainer, PageMain, PageMasthead } from "@/components/layout";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Notice } from "@/components/ui/notice";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { SeverityTag } from "@/components/evals/section/findings-list";
import { SuggestedFix } from "@/components/evals/suggested-fix";
import { evidenceRows, findingKind } from "@/components/evals/findings";
import { formatDate, formatRelativeTime } from "@/lib/formatting";
import type { HealthIssue } from "@/lib/api/types";

export default function FindingPage({ params }: { params: Promise<{ issueId: string }> }) {
  const { issueId } = use(params);
  const ui = useUiTranslations();
  const { data: issue, isLoading, error } = useHealthIssue(issueId);
  usePageTitle(issue?.title ?? ui("Finding"), ui("Findings"), ui("Evals"));

  if (!isLoading && (error || !issue)) {
    return (
      <ResourceNotFound
        title={ui("Finding not found")}
        description={ui(
          "This finding may have been closed, moved to another organization, or the URL may be wrong.",
        )}
        backHref="/evals?tab=findings"
        backLabel="Back to findings"
        resourceId={issueId}
      />
    );
  }

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: ui("Evals"), href: "/evals" },
          { label: ui("Findings"), href: "/evals?tab=findings" },
          { label: issue?.title ?? ui("Finding") },
        ]}
      />
      {!issue ? (
        <PageMain>
          <Skeleton className="h-96 w-full" />
        </PageMain>
      ) : (
        <Finding issue={issue} />
      )}
    </PageContainer>
  );
}

function Finding({ issue }: { issue: HealthIssue }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const { can } = usePolicies("agents");
  const [dismissing, setDismissing] = useState(false);
  const open = issue.status === "open" || issue.status === "needs_check";
  const dismissable = open && !!issue.subject_id && can("agent.manage");
  const evidence = evidenceRows(issue.evidence);
  const investigation = issue.investigation;

  return (
    <>
      <PageMasthead
        icon={<EvalsIcon />}
        title={issue.title}
        description={issue.body}
        actions={
          <div className="flex items-center gap-2">
            {issue.session_id && (
              <LinkButton variant="outline" href={`/sessions/${issue.session_id}`}>
                <ExternalLink className="mr-2 h-4 w-4" />
                {ui("Open session")}
              </LinkButton>
            )}
            {dismissable && (
              <Button variant="ghost" onClick={() => setDismissing(true)}>
                {ui("Dismiss")}
              </Button>
            )}
          </div>
        }
      />
      <PageMain className="space-y-4">
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <SeverityTag issue={issue} />
          <span className="border px-1.5 py-0.5 font-medium text-foreground">
            {ui(findingKind(issue))}
          </span>
          <span>
            {[
              ui("Detected {date}", { date: formatDate(issue.first_detected_at, displayLocale) }),
              issue.agent_name,
              open ? null : ui("Closed"),
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </div>
        {investigation?.dismissed_at && (
          <Notice>
            <p className="text-sm">
              {ui("Dismissed {when}: {reason}", {
                when: formatRelativeTime(investigation.dismissed_at, displayLocale),
                reason: investigation.dismiss_reason ?? "",
              })}
            </p>
          </Notice>
        )}
        <div className="grid gap-4 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
          <div className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle className="text-sm">{ui("Evidence")}</CardTitle>
              </CardHeader>
              <CardContent>
                {evidence.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    {ui("This detector recorded no numbers.")}
                  </p>
                ) : (
                  <dl className="grid grid-cols-[max-content_1fr] gap-x-8 gap-y-2 text-sm">
                    {evidence.map((row) => (
                      <div key={row.label} className="contents">
                        <dt className="text-muted-foreground">{ui(row.label)}</dt>
                        <dd>{row.value}</dd>
                      </div>
                    ))}
                  </dl>
                )}
              </CardContent>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle className="text-sm">{ui("What happened")}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-sm">
                <InspectorAccount issue={issue} />
              </CardContent>
            </Card>
            <SuggestedFix issueId={issue.id} />
          </div>
          {investigation?.cluster_title && (
            <Card className="self-start">
              <CardHeader>
                <CardTitle className="text-sm">{ui("Similar findings")}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-1 text-sm">
                <p className="font-medium">{investigation.cluster_title}</p>
                <p className="text-muted-foreground">
                  {(investigation.cluster_open ?? 1) <= 1
                    ? ui("No other open finding looks like this one.")
                    : ui("{count} open findings, this one included, describe the same problem.", {
                        count: investigation.cluster_open ?? 1,
                      })}
                </p>
              </CardContent>
            </Card>
          )}
        </div>
      </PageMain>
      <DismissDialog issue={issue} open={dismissing} onOpenChange={setDismissing} />
    </>
  );
}

function InspectorAccount({ issue }: { issue: HealthIssue }) {
  const ui = useUiTranslations();
  const investigation = issue.investigation;
  if (!investigation || investigation.status === "running") {
    return (
      <p className="text-muted-foreground">
        {ui(
          "The Inspector has not written this finding up yet. It usually does within 15 minutes.",
        )}
      </p>
    );
  }
  if (investigation.status === "unavailable") {
    return (
      <p className="text-muted-foreground">
        {ui("The Inspector needs a default model in this organization to write findings up.")}
      </p>
    );
  }
  if (investigation.status === "failed" || !investigation.what_happened) {
    return (
      <p className="text-muted-foreground">
        {ui("The Inspector could not write this finding up. It will try again later.")}
      </p>
    );
  }
  return (
    <>
      <p>{investigation.what_happened}</p>
      {investigation.suggested_action && (
        <p>
          <span className="font-medium">{ui("Next step")}: </span>
          {investigation.suggested_action}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        {ui("Written by the Inspector from the session and the detector's numbers.")}
      </p>
    </>
  );
}

function DismissDialog({
  issue,
  open,
  onOpenChange,
}: {
  issue: HealthIssue;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const ui = useUiTranslations();
  const router = useRouter();
  const dismiss = useDismissFinding();
  const [reason, setReason] = useState("");
  const submit = async () => {
    await dismiss.mutateAsync({ id: issue.id, reason: reason.trim() });
    onOpenChange(false);
    router.push("/evals?tab=findings");
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{ui("Dismiss finding")}</DialogTitle>
          <DialogDescription>
            {ui(
              "Say why it needs nothing: a false alarm, expected work, or already handled. The reason is kept to tune the detectors.",
            )}
          </DialogDescription>
        </DialogHeader>
        <Textarea
          aria-label={ui("Reason")}
          value={reason}
          maxLength={500}
          onChange={(e) => setReason(e.target.value)}
          placeholder={ui("Expected: this is our nightly load test")}
        />
        {dismiss.error && (
          <p className="text-sm text-destructive">{(dismiss.error as Error).message}</p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {ui("Cancel")}
          </Button>
          <Button onClick={submit} disabled={!reason.trim() || dismiss.isPending}>
            {ui("Dismiss")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
