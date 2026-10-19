"use client";

import { Check, ExternalLink, ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { ApprovalEpisode } from "@/lib/approval-episodes";
import { cn } from "@/lib/utils";

function sameWords(left: string | undefined, right: string | undefined): boolean {
  const normalize = (value: string | undefined) =>
    (value ?? "").trim().toLowerCase().replace(/\s+/g, " ");
  const normalized = normalize(left);
  return normalized.length > 0 && normalized === normalize(right);
}

function readableTimestamp(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function ApprovalEpisodeCard({
  episode,
  sessionId,
}: {
  episode: ApprovalEpisode;
  sessionId: string;
}) {
  const approved = episode.status === "approved";
  const title = episode.grant?.action ?? episode.ask?.action ?? "Critical action";
  const showAskedAction = Boolean(
    episode.ask && episode.grant && !sameWords(episode.ask.action, episode.grant.action),
  );
  const consentHref = episode.grant?.consentMessageId ? `/sessions/${sessionId}/trace` : undefined;

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 py-4">
        <div className="flex items-start gap-3">
          <span
            className={cn(
              "mt-0.5 inline-flex size-8 shrink-0 items-center justify-center border",
              approved
                ? "border-success/30 bg-success/12 text-success"
                : "border-border bg-muted text-muted-foreground",
            )}
          >
            {approved ? <Check className="size-4" /> : <ShieldCheck className="size-4" />}
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-start justify-between gap-3">
              <h2 className="min-w-0 flex-1 font-medium text-foreground">{title}</h2>
              <Badge variant={approved ? "success" : "outline"} className="mt-0.5">
                {approved ? "Approved" : "Open"}
              </Badge>
            </div>
            {episode.grant?.detail && !sameWords(episode.grant.detail, title) && (
              <p className="mt-1 whitespace-pre-wrap text-sm text-muted-foreground">
                {episode.grant.detail}
              </p>
            )}
            {!episode.grant && episode.ask?.question && !sameWords(episode.ask.question, title) && (
              <p className="mt-1 whitespace-pre-wrap text-sm text-muted-foreground">
                {episode.ask.question}
              </p>
            )}
            {episode.recordedWithoutRequest && (
              <p className="mt-2 text-sm text-muted-foreground">
                Recorded without a logged request.
              </p>
            )}
          </div>
        </div>

        {episode.ask && episode.grant && (
          <div className="border border-border/70 bg-muted/40 px-3 py-2.5 sm:ml-11">
            <p className="text-xs font-medium uppercase tracking-[0.12em] text-muted-foreground">
              Requested
            </p>
            {showAskedAction && (
              <p className="mt-1 whitespace-pre-wrap text-sm text-foreground">
                {episode.ask.action}
              </p>
            )}
            {episode.ask.question && !sameWords(episode.ask.question, episode.ask.action) && (
              <p className="mt-1 whitespace-pre-wrap text-sm text-muted-foreground">
                {episode.ask.question}
              </p>
            )}
            <p className="mt-1 text-xs text-muted-foreground">
              Asked {readableTimestamp(episode.ask.at)}
            </p>
          </div>
        )}

        <div className="flex flex-col gap-2 sm:ml-11 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-xs text-muted-foreground">
            {episode.grant ? (
              <>
                Approved by {episode.grant.approvedBy}
                <span aria-hidden> · </span>
                {readableTimestamp(episode.grant.at)}
              </>
            ) : episode.awaitingConsent ? (
              "Waiting for consent. Nothing has been recorded as approved."
            ) : (
              "Nothing was recorded as approved."
            )}
          </p>
          {consentHref && (
            <LinkButton href={consentHref} variant="outline" size="sm" className="shrink-0">
              View consent
              <ExternalLink className="size-3.5" />
            </LinkButton>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

export function SessionApprovals({
  sessionId,
  episodes,
  loading,
  error,
  onRetry,
}: {
  sessionId: string;
  episodes: ApprovalEpisode[];
  loading?: boolean;
  error?: string;
  onRetry?: () => void;
}) {
  const approvedCount = episodes.filter((episode) => episode.status === "approved").length;
  const openCount = episodes.length - approvedCount;

  if (loading) {
    return <p className="py-12 text-center text-sm text-muted-foreground">Loading approvals…</p>;
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4">
      <p className="text-sm text-muted-foreground">
        Each card is one approval: the request, and what was recorded as approved.
      </p>

      {error && (
        <Card>
          <CardContent className="py-6 text-center">
            <p className="font-medium">Couldn’t load the full approval history</p>
            <p className="mt-1 text-sm text-muted-foreground">{error}</p>
            {onRetry && (
              <Button className="mt-4" variant="outline" onClick={onRetry}>
                Try again
              </Button>
            )}
          </CardContent>
        </Card>
      )}

      {episodes.length === 0 && !error && (
        <Card>
          <CardContent className="py-12 text-center">
            <ShieldCheck className="mx-auto size-8 text-muted-foreground/60" />
            <p className="mt-3 font-medium">No approvals in this session</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Requests and the approvals recorded for them will show up here together.
            </p>
          </CardContent>
        </Card>
      )}

      {episodes.length > 0 && (
        <>
          <p className="text-xs text-muted-foreground">
            {approvedCount} approved
            {openCount > 0 ? ` · ${openCount} open` : ""}
          </p>
          <div className="space-y-3">
            {episodes.map((episode) => (
              <ApprovalEpisodeCard key={episode.id} episode={episode} sessionId={sessionId} />
            ))}
          </div>
        </>
      )}
    </div>
  );
}
