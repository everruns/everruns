"use client";

// Settings → Limits: org-wide limits on unattended work (schedules, triggers,
// retries). Admins edit; everyone else reads. PATCH replaces the whole
// `background_limits` object, so the form always sends every field.

import { useUiTranslations } from "@/providers/locale-provider";
import { useState } from "react";
import { z } from "zod";
import { Loader2 } from "lucide-react";
import { usePageSettingsDraft } from "@/components/chat-mode/use-page-settings-draft";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useUpdateOrganization } from "@/hooks/use-organizations";
import { BACKGROUND_LIMIT_CEILINGS, effectiveBackgroundLimits } from "@/lib/background-limits";
import type { BackgroundLimits, Organization } from "@/lib/api/types";

const limitsSchema = z.strictObject({
  schedule_max_lifetime_days: z.number(),
  schedule_max_runs: z.number(),
  schedule_budget_usd: z.number(),
  schedule_max_budget_usd: z.number(),
  org_unattended_daily_usd: z.number().nullable(),
});

type LimitsForm = {
  schedule_max_lifetime_days: string;
  schedule_max_runs: string;
  schedule_budget_usd: string;
  schedule_max_budget_usd: string;
  org_unattended_daily_usd: string;
  noDailyCap: boolean;
};

function toForm(limits: BackgroundLimits): LimitsForm {
  return {
    schedule_max_lifetime_days: String(limits.schedule_max_lifetime_days),
    schedule_max_runs: String(limits.schedule_max_runs),
    schedule_budget_usd: String(limits.schedule_budget_usd),
    schedule_max_budget_usd: String(limits.schedule_max_budget_usd),
    org_unattended_daily_usd:
      limits.org_unattended_daily_usd === null ? "" : String(limits.org_unattended_daily_usd),
    noDailyCap: limits.org_unattended_daily_usd === null,
  };
}

/** Parses the form, or returns the English source text of the first problem. */
export function parseLimitsForm(form: LimitsForm): BackgroundLimits | string {
  const days = Number(form.schedule_max_lifetime_days);
  if (
    !Number.isInteger(days) ||
    days < 1 ||
    days > BACKGROUND_LIMIT_CEILINGS.schedule_max_lifetime_days
  ) {
    return "Schedule lifetime must be a whole number of days from 1 to 7.";
  }
  const runs = Number(form.schedule_max_runs);
  if (!Number.isInteger(runs) || runs < 1 || runs > BACKGROUND_LIMIT_CEILINGS.schedule_max_runs) {
    return "Runs per schedule must be a whole number from 1 to 96.";
  }
  const budget = Number(form.schedule_budget_usd);
  const maxBudget = Number(form.schedule_max_budget_usd);
  if (!form.schedule_budget_usd.trim() || !Number.isFinite(budget) || budget <= 0) {
    return "Default budget per schedule must be more than $0.";
  }
  if (!form.schedule_max_budget_usd.trim() || !Number.isFinite(maxBudget) || maxBudget <= 0) {
    return "Largest schedule budget must be more than $0.";
  }
  if (budget > maxBudget) {
    return "Default budget per schedule cannot be more than the largest budget.";
  }
  let daily: number | null = null;
  if (!form.noDailyCap) {
    daily = Number(form.org_unattended_daily_usd);
    if (!form.org_unattended_daily_usd.trim() || !Number.isFinite(daily) || daily <= 0) {
      return "Daily cap must be more than $0, or turn on No cap.";
    }
  }
  return {
    schedule_max_lifetime_days: days,
    schedule_max_runs: runs,
    schedule_budget_usd: budget,
    schedule_max_budget_usd: maxBudget,
    org_unattended_daily_usd: daily,
  };
}

export function BackgroundLimitsSettings({
  organization,
  canManage,
}: {
  organization: Organization;
  canManage: boolean;
}) {
  const ui = useUiTranslations();
  const update = useUpdateOrganization();
  const stored = effectiveBackgroundLimits(organization.background_limits);
  const [form, setForm] = useState<LimitsForm>(() => toForm(stored));
  const [savedForm, setSavedForm] = useState<LimitsForm>(() => toForm(stored));
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const draft = usePageSettingsDraft({
    values: stored,
    schema: limitsSchema,
    entityType: "organization",
    entityId: organization.id,
    formId: "background_limits",
    label: ui("Limits"),
    writable: canManage && !update.isPending,
    save: (values) => update.mutateAsync({ background_limits: values }),
  });

  const shown = draft.adopted ? toForm(draft.values) : form;
  const dirty = JSON.stringify(form) !== JSON.stringify(savedForm);
  const readOnly = !canManage || update.isPending || draft.adopted;

  const setField = (field: keyof LimitsForm, value: string | boolean) => {
    setForm((current) => ({ ...current, [field]: value }));
    setSaved(false);
    setError(null);
  };

  async function save() {
    if (!canManage || update.isPending) return;
    const parsed = parseLimitsForm(form);
    if (typeof parsed === "string") {
      setError(ui(parsed));
      return;
    }
    setError(null);
    try {
      const result = await update.mutateAsync({ background_limits: parsed });
      const next = toForm(effectiveBackgroundLimits(result.background_limits ?? parsed));
      setForm(next);
      setSavedForm(next);
      setSaved(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not save limits."));
    }
  }

  const numberField = (
    field: Exclude<keyof LimitsForm, "noDailyCap">,
    label: string,
    hint: string,
    attrs: { min: number; max?: number; step: number; prefix?: string; suffix?: string },
    disabled = readOnly,
  ) => (
    <div className="space-y-1.5">
      <Label htmlFor={`limits-${field}`}>{label}</Label>
      <div className="flex max-w-xs items-center gap-2">
        {attrs.prefix && <span className="text-sm text-muted-foreground">{attrs.prefix}</span>}
        <Input
          id={`limits-${field}`}
          type="number"
          inputMode="decimal"
          min={attrs.min}
          max={attrs.max}
          step={attrs.step}
          value={shown[field]}
          onChange={(event) => setField(field, event.target.value)}
          readOnly={disabled}
          disabled={disabled && !readOnly}
          aria-describedby={`limits-${field}-hint`}
        />
        {attrs.suffix && <span className="text-sm text-muted-foreground">{attrs.suffix}</span>}
      </div>
      <p id={`limits-${field}-hint`} className="text-xs text-muted-foreground">
        {hint}
      </p>
    </div>
  );

  return (
    <Card className="space-y-6 p-6">
      <div className="space-y-1">
        <h2 className="font-medium">{ui("Unattended work")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui(
            "Limits for work no person started: schedules agents create, trigger runs and automatic retries. A person can still extend a schedule from its session.",
          )}
        </p>
      </div>
      {numberField(
        "schedule_max_lifetime_days",
        ui("Longest an agent's schedule may run"),
        ui("Up to 7 days. The agent must set an end, and it cannot ask for more than this."),
        {
          min: 1,
          max: BACKGROUND_LIMIT_CEILINGS.schedule_max_lifetime_days,
          step: 1,
          suffix: ui("days"),
        },
      )}
      {numberField(
        "schedule_max_runs",
        ui("Most runs per agent schedule"),
        ui(
          "Up to 96 runs. The schedule stops at this count or at its end time, whichever is first.",
        ),
        { min: 1, max: BACKGROUND_LIMIT_CEILINGS.schedule_max_runs, step: 1, suffix: ui("runs") },
      )}
      {numberField(
        "schedule_budget_usd",
        ui("Default budget per schedule"),
        ui("What a schedule may spend in total when the agent does not ask for a budget."),
        { min: 0.01, step: 0.01, prefix: "$" },
      )}
      {numberField(
        "schedule_max_budget_usd",
        ui("Largest budget an agent may give a schedule"),
        ui("The most an agent may ask for. A person can raise a single schedule past it."),
        { min: 0.01, step: 0.01, prefix: "$" },
      )}
      <div className="space-y-3">
        {numberField(
          "org_unattended_daily_usd",
          ui("Daily cap on unattended work (schedules, triggers, retries)"),
          ui("What all unattended work in this organization may spend per day, together."),
          { min: 0.01, step: 0.01, prefix: "$" },
          readOnly || shown.noDailyCap,
        )}
        <div className="flex items-center gap-2">
          <Switch
            id="limits-no-daily-cap"
            checked={shown.noDailyCap}
            onCheckedChange={(checked) => setField("noDailyCap", checked)}
            disabled={readOnly}
          />
          <Label htmlFor="limits-no-daily-cap">{ui("No cap")}</Label>
        </div>
      </div>
      {draft.controls}
      {!canManage && (
        <p className="text-sm text-muted-foreground">
          {ui("Only organization owners and admins can change these limits.")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {canManage && !draft.adopted && (
        <div className="flex items-center gap-3">
          <Button onClick={save} disabled={update.isPending || !dirty}>
            {update.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
            {ui("Save")}
          </Button>
          {saved && !dirty && (
            <span role="status" className="text-sm text-success">
              {ui("Saved")}
            </span>
          )}
        </div>
      )}
    </Card>
  );
}
