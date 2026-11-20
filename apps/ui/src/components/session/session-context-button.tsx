"use client";

import { useState } from "react";
import { Layers } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { useSessionContextReport } from "@/hooks/use-sessions";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { formatTokens } from "@/lib/formatting";
import type { SessionContextReport } from "@/lib/api/types";
import { compactionStrategyLabels } from "@/lib/compaction";

export function SessionContextDetails({ report }: { report: SessionContextReport }) {
  const ui = useUiTranslations();
  const policy = report.compaction;
  const checkpoint = policy?.checkpoint;
  return (
    <div className="space-y-5 text-sm">
      <section className="space-y-2">
        <h3 className="font-medium">{ui("Last model context")}</h3>
        {report.measurement_available === false ? (
          <p className="text-muted-foreground">{ui("No recent model context measurement.")}</p>
        ) : (
          <>
            <p className="text-2xl font-semibold tabular-nums">
              {formatTokens(report.estimated_input_tokens)}{" "}
              <span className="text-xs font-normal text-muted-foreground">
                {ui("estimated tokens")}
              </span>
            </p>
            <p className="break-all text-xs text-muted-foreground">
              {report.model}
              {report.context_window_tokens
                ? ` · ${formatTokens(report.context_window_tokens)} ${ui("Model window")}`
                : ""}
            </p>
            <p className="text-xs text-muted-foreground">
              {ui("Last recorded request; the next request may differ.")}
            </p>
          </>
        )}
      </section>
      {policy && (
        <section className="space-y-3 border-t pt-4">
          <h3 className="font-medium">{ui("Effective compaction policy")}</h3>
          <dl className="grid grid-cols-2 gap-x-4 gap-y-2">
            <dt className="text-muted-foreground">{ui("Strategy")}</dt>
            <dd>
              {policy.enabled
                ? compactionStrategyLabels(policy.strategy ?? "auto")
                    .map((label) => ui(label))
                    .join(" + ")
                : ui("Disabled")}
            </dd>
            <dt className="text-muted-foreground">{ui("Working history budget")}</dt>
            <dd>
              {policy.context_budget_tokens ? formatTokens(policy.context_budget_tokens) : "—"}
            </dd>
            <dt className="text-muted-foreground">{ui("Proactive compaction")}</dt>
            <dd>{ui(policy.proactive ? "Enabled" : "Disabled")}</dd>
            <dt className="text-muted-foreground">{ui("History retrieval")}</dt>
            <dd>{ui(policy.history_retrieval_available ? "Available" : "Unavailable")}</dd>
          </dl>
          <p className="text-xs text-muted-foreground">
            {ui(
              "Keeps recent work in context and older history retrievable. Summary calls may add cost.",
            )}
          </p>
        </section>
      )}
      <section className="space-y-2 border-t pt-4">
        <h3 className="font-medium">{ui("Saved checkpoint")}</h3>
        {checkpoint ? (
          <>
            <p>
              {ui(
                (
                  {
                    portable_summary: "Portable summary",
                    legacy_summary: "Summary",
                    legacy_trim: "History trimming",
                  } as Record<string, string>
                )[checkpoint.kind] ?? "Provider-native",
              )}
            </p>
            <p className="text-xs text-muted-foreground">
              {ui("History through event {sequence}", { sequence: checkpoint.source_sequence })}
            </p>
            {checkpoint.provider && (
              <p className="break-all text-xs text-muted-foreground">
                {checkpoint.provider} · {checkpoint.model}
              </p>
            )}
          </>
        ) : (
          <p className="text-muted-foreground">
            {ui(
              policy?.checkpoint_store_available === false
                ? "Checkpoint storage is unavailable."
                : policy?.checkpoint_store_available == null
                  ? "Checkpoint storage has not been reported by this host."
                  : "No checkpoint has been saved yet.",
            )}
          </p>
        )}
        <p className="text-xs text-muted-foreground">
          {ui(
            "Original session history is retained. Provider-native checkpoint contents stay private.",
          )}
        </p>
        {policy?.last_event?.checkpoint_id && (
          <p className="text-xs text-muted-foreground">
            {ui("Checkpoint saved")}
            {policy.last_event.source_sequence != null &&
              ` · ${ui("History through event {sequence}", { sequence: policy.last_event.source_sequence })}`}
          </p>
        )}
        {policy?.last_event && (
          <p className="text-xs text-muted-foreground">
            {ui("Last compaction")}:{" "}
            {ui(
              (
                {
                  compacting: "Compacting context…",
                  completed: "Completed",
                  skipped: "No smaller context was installed.",
                  failed: "Compaction failed. Previous history was retained.",
                } as Record<string, string>
              )[policy.last_event.state] ?? "Context reduction",
            )}
          </p>
        )}
      </section>
    </div>
  );
}

export function SessionContextButton({ sessionId }: { sessionId: string }) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="outline" size="sm" aria-label={ui("Context")} onClick={() => setOpen(true)}>
        <Layers className="h-4 w-4" />
        <span className="hidden sm:inline">{ui("Context")}</span>
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85dvh] overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{ui("Context")}</DialogTitle>
            <DialogDescription>
              {ui("Effective policy and saved context, without changing the conversation.")}
            </DialogDescription>
          </DialogHeader>
          {open && <SessionContextDialogBody sessionId={sessionId} />}
        </DialogContent>
      </Dialog>
    </>
  );
}

function SessionContextDialogBody({ sessionId }: { sessionId: string }) {
  const ui = useUiTranslations();
  const query = useSessionContextReport(sessionId);
  return query.isLoading ? (
    <p role="status">{ui("Loading")}</p>
  ) : query.isError ? (
    <div role="alert" className="space-y-3">
      <p>{ui("Could not load context details.")}</p>
      <Button variant="outline" onClick={() => query.refetch()}>
        {ui("Try again")}
      </Button>
    </div>
  ) : query.data ? (
    <SessionContextDetails report={query.data} />
  ) : null;
}
