"use client";

// A person extends a schedule's end time, run count or budget. People may go
// past the ceilings agents are held to; the server still validates the values.

import { useUiTranslations } from "@/providers/locale-provider";
import { useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useUpdateSessionSchedule } from "@/hooks/use-session-schedules";
import type { SessionSchedule, UpdateSessionScheduleRequest } from "@/lib/api/types";

/** `datetime-local` wants local wall-clock time without a zone. */
function toLocalInput(iso: string | null | undefined): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

interface ExtendForm {
  endsAt: string;
  maxRuns: string;
  budgetUsd: string;
}

/** Builds the PATCH body, or returns the English source text of the first problem. */
export function buildExtendRequest(
  schedule: Pick<SessionSchedule, "ends_at" | "max_runs" | "budget">,
  form: ExtendForm,
  now: Date = new Date(),
): UpdateSessionScheduleRequest | string {
  const request: UpdateSessionScheduleRequest = {};
  if (form.endsAt && form.endsAt !== toLocalInput(schedule.ends_at)) {
    const endsAt = new Date(form.endsAt);
    if (Number.isNaN(endsAt.getTime()) || endsAt <= now) {
      return "The end time must be in the future.";
    }
    request.ends_at = endsAt.toISOString();
  }
  if (form.maxRuns && form.maxRuns !== String(schedule.max_runs ?? "")) {
    const runs = Number(form.maxRuns);
    if (!Number.isInteger(runs) || runs < 1) return "Runs must be a whole number above 0.";
    request.max_runs = runs;
  }
  if (form.budgetUsd && form.budgetUsd !== String(schedule.budget?.limit_usd ?? "")) {
    const budget = Number(form.budgetUsd);
    if (!Number.isFinite(budget) || budget <= 0) return "The budget must be more than $0.";
    request.budget_usd = budget;
  }
  if (Object.keys(request).length === 0) return "Change at least one value to extend.";
  return request;
}

export function SessionScheduleExtendDialog({
  schedule,
  open,
  onOpenChange,
}: {
  schedule: SessionSchedule;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const ui = useUiTranslations();
  const update = useUpdateSessionSchedule();
  // The caller mounts the dialog per opening, so the form starts from current values.
  const [form, setForm] = useState<ExtendForm>(() => ({
    endsAt: toLocalInput(schedule.ends_at),
    maxRuns: schedule.max_runs == null ? "" : String(schedule.max_runs),
    budgetUsd: schedule.budget ? String(schedule.budget.limit_usd) : "",
  }));
  const [error, setError] = useState<string | null>(null);

  const change = (field: keyof ExtendForm, value: string) => {
    setForm((current) => ({ ...current, [field]: value }));
    setError(null);
  };

  async function submit() {
    const request = buildExtendRequest(schedule, form);
    if (typeof request === "string") {
      setError(ui(request));
      return;
    }
    try {
      await update.mutateAsync({
        sessionId: schedule.session_id,
        scheduleId: schedule.id,
        request,
      });
      onOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not extend the schedule."));
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{ui("Extend schedule")}</DialogTitle>
          <DialogDescription>
            {ui(
              "Give this schedule more time, runs or budget. You can go past the limits agents are held to.",
            )}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor={`extend-ends-at-${schedule.id}`}>{ui("Ends at")}</Label>
            <Input
              id={`extend-ends-at-${schedule.id}`}
              type="datetime-local"
              value={form.endsAt}
              onChange={(event) => change("endsAt", event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={`extend-max-runs-${schedule.id}`}>{ui("Most runs")}</Label>
            <Input
              id={`extend-max-runs-${schedule.id}`}
              type="number"
              min={1}
              step={1}
              value={form.maxRuns}
              onChange={(event) => change("maxRuns", event.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              {ui("Runs so far: {value0}", { value0: schedule.trigger_count })}
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={`extend-budget-${schedule.id}`}>{ui("Budget (USD)")}</Label>
            <Input
              id={`extend-budget-${schedule.id}`}
              type="number"
              min={0.01}
              step={0.01}
              value={form.budgetUsd}
              onChange={(event) => change("budgetUsd", event.target.value)}
            />
          </div>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {ui("Cancel")}
          </Button>
          <Button onClick={submit} disabled={update.isPending}>
            {update.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
            {ui("Extend")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
