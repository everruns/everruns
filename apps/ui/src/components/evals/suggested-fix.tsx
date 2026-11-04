"use client";

// The finding's suggested fix (approved mockup 04, "Suggested fix"): the text
// the Inspector would add to the agent's instructions, the check that runs the
// agent's eval with and without it, and Approve once that check shows no
// regressions. Approve adds the text as a new agent revision; nothing changes
// the agent without it (knowledge/evaluation/suggested-fixes.md).

import Link from "next/link";
import { useState } from "react";
import { Check } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { useFindingFix, useFindingFixAction } from "@/hooks/use-evals";
import { usePolicies } from "@/hooks/use-policies";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";
import type { FindingFix } from "@/lib/api/types";

const ADDITION_CHARS = 1500;

type BadgeVariant = "accent" | "secondary" | "destructive" | "success" | "outline";

export function fixBadge(fix: FindingFix): { label: string; variant: BadgeVariant } {
  switch (fix.status) {
    case "verified":
      return fix.current
        ? { label: "Needs approval", variant: "accent" }
        : { label: "Check again", variant: "secondary" };
    case "checking":
      return { label: "Checking", variant: "secondary" };
    case "regressed":
      return { label: "Breaks cases", variant: "destructive" };
    case "check_failed":
      return { label: "Check failed", variant: "destructive" };
    case "applied":
      return { label: "Applied", variant: "success" };
    case "rejected":
      return { label: "Rejected", variant: "outline" };
    default:
      return { label: "Not checked", variant: "secondary" };
  }
}

export function SuggestedFix({ issueId }: { issueId: string }) {
  const { data: fix } = useFindingFix(issueId);
  if (!fix) return null;
  return <FixCard issueId={issueId} fix={fix} />;
}

function FixCard({ issueId, fix }: { issueId: string; fix: FindingFix }) {
  const ui = useUiTranslations();
  const { can } = usePolicies("agents");
  const action = useFindingFixAction(issueId);
  const [editing, setEditing] = useState(false);
  const [rejecting, setRejecting] = useState(false);
  const [draft, setDraft] = useState(fix.addition);
  const [note, setNote] = useState("");
  const badge = fixBadge(fix);
  const decided = fix.status === "applied" || fix.status === "rejected";
  const canManage = can("agent.manage") && !decided;
  const approvable = fix.status === "verified" && fix.current;
  const checkable =
    ["proposed", "regressed", "check_failed"].includes(fix.status) ||
    (fix.status === "verified" && !fix.current);
  const busy = action.isPending;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          {ui("Suggested fix")}
          <Badge variant={badge.variant}>{ui(badge.label)}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">
        <p>{fix.summary}</p>
        {editing ? (
          <Textarea
            aria-label={ui("Text to add to the agent's instructions")}
            className="font-mono text-xs"
            value={draft}
            maxLength={ADDITION_CHARS}
            onChange={(e) => setDraft(e.target.value)}
          />
        ) : (
          <pre className="whitespace-pre-wrap border bg-muted/40 px-3 py-2 font-mono text-xs">
            + {fix.addition}
          </pre>
        )}
        <CheckLine fix={fix} />
        {rejecting && (
          <Textarea
            aria-label={ui("Why not")}
            value={note}
            maxLength={500}
            onChange={(e) => setNote(e.target.value)}
            placeholder={ui("The schedule itself is wrong; fixing it there")}
          />
        )}
        {action.error && (
          <p className="text-sm text-destructive">{(action.error as Error).message}</p>
        )}
        {canManage && (
          <div className="flex flex-wrap items-center gap-2">
            {editing ? (
              <>
                <Button
                  size="sm"
                  disabled={busy || !draft.trim()}
                  onClick={() =>
                    action.mutate(
                      { kind: "edit", fixId: fix.id, addition: draft.trim() },
                      { onSuccess: () => setEditing(false) },
                    )
                  }
                >
                  {ui("Save")}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
                  {ui("Cancel")}
                </Button>
              </>
            ) : rejecting ? (
              <>
                <Button
                  size="sm"
                  disabled={busy || !note.trim()}
                  onClick={() =>
                    action.mutate(
                      { kind: "reject", fixId: fix.id, note: note.trim() },
                      { onSuccess: () => setRejecting(false) },
                    )
                  }
                >
                  {ui("Reject")}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setRejecting(false)}>
                  {ui("Cancel")}
                </Button>
              </>
            ) : (
              <>
                {approvable && (
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() =>
                      action.mutate({
                        kind: "apply",
                        fixId: fix.id,
                        reason: fix.summary.slice(0, 200),
                      })
                    }
                  >
                    <Check className="mr-1 h-4 w-4" />
                    {ui("Approve")}
                  </Button>
                )}
                {checkable && (
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => action.mutate({ kind: "check", fixId: fix.id })}
                  >
                    {fix.status === "proposed" ? ui("Check against evals") : ui("Check again")}
                  </Button>
                )}
                {fix.status !== "checking" && (
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      setDraft(fix.addition);
                      setEditing(true);
                    }}
                  >
                    {ui("Edit")}
                  </Button>
                )}
                <Button size="sm" variant="ghost" onClick={() => setRejecting(true)}>
                  {ui("Reject")}
                </Button>
              </>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function CheckLine({ fix }: { fix: FindingFix }) {
  const ui = useUiTranslations();
  const experimentHref =
    fix.eval_id && fix.experiment_id
      ? `/evals/${fix.eval_id}/experiments/${fix.experiment_id}`
      : null;
  const details = experimentHref && (
    <>
      {" "}
      <Link className="underline underline-offset-2" href={experimentHref}>
        {ui("Open the check")}
      </Link>
    </>
  );
  const check = fix.check;
  switch (fix.status) {
    case "applied":
      return <p className="text-muted-foreground">{ui("Added to the agent's instructions.")}</p>;
    case "rejected":
      return (
        <p className="text-muted-foreground">
          {ui("Rejected: {note}", { note: fix.decision_note ?? "" })}
        </p>
      );
    case "checking":
      return (
        <p className="text-muted-foreground">
          {ui("Running the agent's eval with and without the fix.")}
          {details}
        </p>
      );
    case "check_failed":
      return (
        <p className="text-muted-foreground">
          {ui("The check did not finish, so nothing was compared. Check it again.")}
          {details}
        </p>
      );
    case "proposed":
      return (
        <p className="text-muted-foreground">
          {ui(
            "Check it on this agent's eval cases first: it can be approved once the check shows no regressions.",
          )}
        </p>
      );
  }
  if (!check) return null;
  const line = ui("Checked on {eval} v{version}: {before}/{cases} → {after}/{cases}", {
    eval: check.eval_name,
    version: check.dataset_version,
    before: check.current.passed,
    after: check.with_fix.passed,
    cases: check.cases,
  });
  return (
    <p className="text-muted-foreground">
      {line}
      {", "}
      {check.broke === 0
        ? ui("no regressions.")
        : ui("{count} cases broke.", { count: check.broke })}
      {!fix.current && fix.status === "verified" && (
        <> {ui("The agent's instructions changed after the check.")}</>
      )}
      {details}
    </p>
  );
}
