"use client";

// One-click repairs the server declares on a health issue
// (knowledge/operations/health-issues.md, "Fixers"). The server decides what a
// fixer does and builds any consent URL; this card only says what will happen,
// runs it, and sends the browser to the provider when approval is needed.
import { useEffect, useState } from "react";
import { ExternalLink, Wand2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useUiTranslations } from "@/providers/locale-provider";
import { useHealthIssueAction, useRunHealthFixer } from "@/hooks/use-health-issues";
import { safeConsentUrl } from "@/lib/api/health-issues";
import type { HealthIssue, IssueFixer } from "@/lib/api/types";

/** What happened after a fixer ran, for the card to say. */
export type FixState =
  | { phase: "idle" }
  | { phase: "away" } // approving in another tab; checked again on return
  | { phase: "done" }
  | { phase: "error"; message: string };

/**
 * Runs a fixer and follows its outcome. A consent that returns by itself
 * replaces this page; one that does not opens in a new tab, and the issue is
 * checked again when the person comes back to this one.
 */
export function useStartFix(issueId: string) {
  const ui = useUiTranslations();
  const run = useRunHealthFixer();
  const check = useHealthIssueAction("check");
  const [state, setState] = useState<FixState>({ phase: "idle" });
  const [awayId, setAwayId] = useState(issueId);
  const { mutate: recheck } = check;

  useEffect(() => {
    if (state.phase !== "away") return;
    const onFocus = () => {
      recheck(awayId);
      setState({ phase: "idle" });
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [state.phase, awayId, recheck]);

  /** Run `kind` on `id` (the hook's issue unless given). */
  const start = async (
    kind: string,
    id: string = issueId,
  ): Promise<"left" | "away" | "done" | "error"> => {
    setState({ phase: "idle" });
    setAwayId(id);
    try {
      const outcome = await run.mutateAsync({ id, kind });
      if (outcome.status === "done") {
        setState({ phase: "done" });
        return "done";
      }
      const url = safeConsentUrl(outcome.consent_url);
      if (!url) throw new Error(ui("Everruns returned an unexpected approval page"));
      if (outcome.returns_automatically) {
        window.location.assign(url.href);
        return "left";
      }
      window.open(url.href, "_blank", "noopener,noreferrer");
      setState({ phase: "away" });
      return "away";
    } catch (error) {
      setState({
        phase: "error",
        message: error instanceof Error ? error.message : ui("The fix did not run"),
      });
      return "error";
    }
  };
  return { start, state, pending: run.isPending };
}

export function HealthFixer({
  issue,
  fixer,
  manualSteps,
}: {
  issue: HealthIssue;
  fixer: IssueFixer;
  /** Instructions to do it by hand, offered beside the fix. */
  manualSteps?: React.ReactNode;
}) {
  const ui = useUiTranslations();
  const { start, state, pending } = useStartFix(issue.id);
  const check = useHealthIssueAction("check");
  const [showManual, setShowManual] = useState(false);
  const available = !fixer.unavailable_reason;
  const consent = fixer.effect === "consent";
  return (
    <div className="space-y-3 border border-border bg-background p-3" aria-label={ui("Fix")}>
      <p className="flex flex-wrap items-center gap-2 text-xs uppercase tracking-wide text-muted-foreground">
        {available && <Wand2 className="size-3" aria-hidden />}
        {available ? ui("Fix available") : ui("Fix not available yet")}
        {available && consent && fixer.needs_provider_admin && (
          <span className="border border-border px-1.5 normal-case tracking-normal">
            {fixer.kind.startsWith("slack.")
              ? ui("Slack approval needed")
              : ui("Approval in the provider needed")}
          </span>
        )}
      </p>
      {available ? (
        <ol className="list-decimal space-y-1 pl-5 text-sm">
          {fixer.steps.map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      ) : (
        <p className="text-sm">{fixer.unavailable_reason}</p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          size="sm"
          disabled={!available || pending}
          onClick={() => void start(fixer.kind)}
        >
          {pending ? ui("Working…") : fixer.label}
          {consent && <ExternalLink className="size-3" aria-hidden />}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={check.isPending || pending}
          onClick={() => check.mutate(issue.id)}
        >
          {check.isPending ? ui("Checking…") : ui("Check again")}
        </Button>
      </div>
      {state.phase === "away" && (
        <p role="status" className="text-sm">
          {ui(
            "Finish in the Slack tab, then come back here. Everruns checks the app again when you return.",
          )}
        </p>
      )}
      {state.phase === "done" && (
        <p role="status" className="text-sm text-success">
          {ui("Done. Everruns checked the issue again.")}
        </p>
      )}
      {state.phase === "error" && (
        <p role="alert" className="text-sm text-destructive">
          {state.message}
        </p>
      )}
      {check.error && (
        <p role="alert" className="text-sm text-destructive">
          {check.error.message}
        </p>
      )}
      {manualSteps &&
        (available ? (
          <div className="text-xs text-muted-foreground">
            <button
              type="button"
              className="underline"
              aria-expanded={showManual}
              onClick={() => setShowManual((v) => !v)}
            >
              {showManual
                ? ui("Hide manual steps")
                : ui("Prefer to do it yourself? Show manual steps")}
            </button>
            {showManual && <div className="mt-2">{manualSteps}</div>}
          </div>
        ) : (
          <div className="text-xs text-muted-foreground">
            <p className="mb-1">{ui("Or fix it by hand:")}</p>
            {manualSteps}
          </div>
        ))}
    </div>
  );
}
