"use client";

// The Eval Builder's proposals on an agent's Evals tab (approved mockup 03,
// "Proposed by Eval Builder"): suggested cases with why each matters, added to
// a dataset or dismissed one by one. Nothing becomes a case until a person
// adds it. "Review" opens the full case: its messages or simulated user, its
// checks and the sessions behind it.

import { useState } from "react";
import Link from "next/link";
import { Check, Loader2, X } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { useAcceptEvalProposal, useDismissEvalProposal } from "@/hooks/use-evals";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { scorerSummary } from "@/components/evals/case-format";
import type { Eval, EvalBuilderRun, EvalCaseProposal, ProposalCategory } from "@/lib/api/types";

/** Proposals shown on the card before "Review all". */
export const CARD_PROPOSALS = 3;

const CATEGORY_LABEL: Record<ProposalCategory, string> = {
  happy_path: "core job",
  edge_case: "edge case",
  failure: "seen failing",
  adversarial: "adversarial",
};

/** One line under a proposal's name: how it runs, what it covers, where it came from. */
export function proposalMeta(
  proposal: EvalCaseProposal,
  ui: (key: string, params?: Record<string, string | number>) => string,
): string {
  const sessions = proposal.source_session_ids.length;
  return [
    proposal.kind === "simulated" ? ui("Simulated") : ui("Scripted"),
    ui(CATEGORY_LABEL[proposal.category]),
    sessions === 0
      ? ui("from instructions")
      : sessions === 1
        ? ui("seen in 1 session")
        : ui("seen in {count} sessions", { count: sessions }),
  ].join(" · ");
}

/** What the card says about the latest run, or nothing when there is nothing to say. */
export function runNotice(
  run: EvalBuilderRun | null | undefined,
  open: number,
  ui: (key: string, params?: Record<string, string | number>) => string,
): string | null {
  if (!run) return null;
  switch (run.status) {
    case "running":
      return ui("Reading the agent and its recent sessions…");
    case "unavailable":
      return ui("The Eval Builder needs a default model in this organization.");
    case "failed":
      return run.error ?? ui("The Eval Builder could not propose cases. Try again.");
    case "done":
      return open === 0 && run.proposals === 0
        ? ui("No new cases this time: the datasets already cover what it found.")
        : null;
    default:
      return null;
  }
}

export function EvalProposalsCard({
  agentId,
  proposals,
  latestRun,
  datasets,
}: {
  agentId: string;
  proposals: EvalCaseProposal[];
  latestRun?: EvalBuilderRun | null;
  datasets: Eval[];
}) {
  const ui = useUiTranslations();
  const accept = useAcceptEvalProposal(agentId);
  const dismiss = useDismissEvalProposal(agentId);
  const [reviewing, setReviewing] = useState<EvalCaseProposal | null>(null);
  const [reviewAll, setReviewAll] = useState(false);
  const notice = runNotice(latestRun, proposals.length, ui);
  const onlyDataset = datasets.length === 1 ? datasets[0] : undefined;

  const add = (proposal: EvalCaseProposal) => {
    if (onlyDataset) {
      accept.mutate({
        proposalId: proposal.id,
        evalId: onlyDataset.id,
        reason: ui("Added from Eval Builder: {reason}", {
          reason: proposal.reason,
        }),
      });
    } else {
      setReviewing(proposal);
    }
  };

  if (proposals.length === 0 && !notice) return null;

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="text-sm">{ui("Proposed by Eval Builder")}</CardTitle>
        {proposals.length > 0 && (
          <Badge variant="outline">{ui("{count} new", { count: proposals.length })}</Badge>
        )}
      </CardHeader>
      <CardContent className="p-0">
        {notice && (
          <p className="flex items-center gap-2 px-4 pb-3 text-xs text-muted-foreground">
            {latestRun?.status === "running" && <Loader2 className="size-3 animate-spin" />}
            {notice}
          </p>
        )}
        <ul className="divide-y border-t">
          {proposals.slice(0, CARD_PROPOSALS).map((proposal) => (
            <li key={proposal.id} className="space-y-2 px-4 py-3">
              <button
                type="button"
                onClick={() => setReviewing(proposal)}
                className="block text-left"
              >
                <span className="block text-sm font-semibold hover:underline">{proposal.name}</span>
                <span className="block text-xs text-muted-foreground">
                  {proposalMeta(proposal, ui)}
                </span>
              </button>
              <div className="flex items-center gap-1">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => add(proposal)}
                  disabled={datasets.length === 0 || accept.isPending}
                  title={datasets.length === 0 ? ui("Create a dataset first") : undefined}
                >
                  <Check className="size-3.5" />
                  {ui("Add")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => dismiss.mutate({ proposalId: proposal.id })}
                  disabled={dismiss.isPending}
                >
                  <X className="size-3.5" />
                  {ui("Dismiss")}
                </Button>
              </div>
            </li>
          ))}
        </ul>
        {proposals.length > CARD_PROPOSALS && (
          <div className="border-t px-4 py-2.5">
            <button
              type="button"
              onClick={() => setReviewAll(true)}
              className="text-sm font-medium hover:underline"
            >
              {ui("Review all {count}", { count: proposals.length })}
            </button>
          </div>
        )}
        {datasets.length === 0 && proposals.length > 0 && (
          <p className="border-t px-4 py-2.5 text-xs text-muted-foreground">
            {ui("Create a dataset for this agent to add these cases.")}
          </p>
        )}
        {(accept.error || dismiss.error) && (
          <p className="border-t px-4 py-2.5 text-xs text-destructive">
            {((accept.error ?? dismiss.error) as Error).message}
          </p>
        )}
      </CardContent>

      {reviewAll && (
        <Dialog open onOpenChange={(open) => !open && setReviewAll(false)}>
          <DialogContent className="max-h-[calc(100svh-2rem)] max-w-xl overflow-y-auto">
            <DialogHeader>
              <DialogTitle>{ui("Proposed by Eval Builder")}</DialogTitle>
              <DialogDescription>
                {ui("Open a case to see its messages and checks, then add it to a dataset.")}
              </DialogDescription>
            </DialogHeader>
            <ul className="divide-y rounded-md border">
              {proposals.map((proposal) => (
                <li key={proposal.id}>
                  <button
                    type="button"
                    onClick={() => {
                      setReviewAll(false);
                      setReviewing(proposal);
                    }}
                    className="block w-full px-3 py-2.5 text-left hover:bg-muted/50"
                  >
                    <span className="block text-sm font-semibold">{proposal.name}</span>
                    <span className="block text-xs text-muted-foreground">
                      {proposalMeta(proposal, ui)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </DialogContent>
        </Dialog>
      )}

      {reviewing && (
        <ProposalDialog
          agentId={agentId}
          proposal={reviewing}
          datasets={datasets}
          onClose={() => setReviewing(null)}
        />
      )}
    </Card>
  );
}

function ProposalDialog({
  agentId,
  proposal,
  datasets,
  onClose,
}: {
  agentId: string;
  proposal: EvalCaseProposal;
  datasets: Eval[];
  onClose: () => void;
}) {
  const ui = useUiTranslations();
  const accept = useAcceptEvalProposal(agentId);
  const dismiss = useDismissEvalProposal(agentId);
  const [evalId, setEvalId] = useState(datasets[0]?.id ?? "");
  const simulated = proposal.case.simulated_user;

  const add = async () => {
    await accept.mutateAsync({
      proposalId: proposal.id,
      evalId,
      reason: ui("Added from Eval Builder: {reason}", {
        reason: proposal.reason,
      }),
    });
    onClose();
  };
  const turnDown = async () => {
    await dismiss.mutateAsync({ proposalId: proposal.id });
    onClose();
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[calc(100svh-2rem)] max-w-xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{proposal.name}</DialogTitle>
          <DialogDescription>{proposalMeta(proposal, ui)}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4 text-sm">
          {proposal.reason && <p>{proposal.reason}</p>}
          {(proposal.case.conversation ?? []).length > 0 && (
            <section className="space-y-1">
              <h3 className="text-xs font-medium uppercase text-muted-foreground">
                {ui("User says")}
              </h3>
              <ol className="space-y-1">
                {(proposal.case.conversation ?? []).map((message, index) => (
                  <li key={index} className="rounded-md border bg-muted/30 px-3 py-2">
                    {message.content}
                  </li>
                ))}
              </ol>
            </section>
          )}
          {simulated && (
            <section className="space-y-1">
              <h3 className="text-xs font-medium uppercase text-muted-foreground">
                {ui("Simulated user")}
              </h3>
              <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
                <dt className="text-muted-foreground">{ui("Persona")}</dt>
                <dd>{simulated.persona}</dd>
                <dt className="text-muted-foreground">{ui("Goal")}</dt>
                <dd>{simulated.goal}</dd>
                {simulated.done_when && (
                  <>
                    <dt className="text-muted-foreground">{ui("Stops when")}</dt>
                    <dd>{simulated.done_when}</dd>
                  </>
                )}
              </dl>
            </section>
          )}
          <section className="space-y-1">
            <h3 className="text-xs font-medium uppercase text-muted-foreground">
              {ui("Passes when")}
            </h3>
            <ul className="list-disc space-y-0.5 pl-5">
              {proposal.case.scorers.map((scorer, index) => {
                // Generated and hand-written scorer types differ only in null vs
                // undefined for optional fields.
                const summary = scorerSummary(scorer as Parameters<typeof scorerSummary>[0]);
                return <li key={index}>{ui(summary.template, summary.params)}</li>;
              })}
            </ul>
          </section>
          {proposal.source_session_ids.length > 0 && (
            <section className="space-y-1">
              <h3 className="text-xs font-medium uppercase text-muted-foreground">
                {ui("Inspired by")}
              </h3>
              <ul className="flex flex-wrap gap-2">
                {proposal.source_session_ids.map((id, index) => (
                  <li key={id}>
                    <Link href={`/sessions/${id}`} className="text-xs font-medium hover:underline">
                      {ui("Session {n}", { n: index + 1 })}
                    </Link>
                  </li>
                ))}
              </ul>
            </section>
          )}
          {datasets.length > 1 && (
            <Field id="proposal-dataset" label={ui("Add to dataset")}>
              <Select value={evalId} onValueChange={setEvalId}>
                <SelectTrigger id="proposal-dataset">
                  <SelectValue placeholder={ui("Choose a dataset")} />
                </SelectTrigger>
                <SelectContent>
                  {datasets.map((ev) => (
                    <SelectItem key={ev.id} value={ev.id}>
                      {ev.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
          )}
          {(accept.error || dismiss.error) && (
            <p className="text-destructive">{((accept.error ?? dismiss.error) as Error).message}</p>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={turnDown} disabled={dismiss.isPending}>
            {ui("Dismiss")}
          </Button>
          <Button
            onClick={add}
            disabled={!evalId || accept.isPending}
            title={datasets.length === 0 ? ui("Create a dataset first") : undefined}
          >
            {datasets.length === 1
              ? ui("Add to {dataset}", { dataset: datasets[0].name })
              : ui("Add")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
