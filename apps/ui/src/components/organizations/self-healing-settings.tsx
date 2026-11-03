"use client";

// Settings → Limits: what Everruns does on its own when a finding opens
// (knowledge/evaluation/self-healing.md). Admins edit; everyone else reads.
// PATCH replaces the whole `self_healing` object, so the form sends every
// field. Agent changes are suggest or off only: a person approves each one.

import { useState } from "react";
import { Loader2 } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { useUpdateOrganization } from "@/hooks/use-organizations";
import type { Organization, SelfHealingPolicy } from "@/lib/api/types";

const DEFAULTS: SelfHealingPolicy = { runaway_schedules: "auto", agent_fixes: "suggest" };

export function SelfHealingSettings({
  organization,
  canManage,
}: {
  organization: Organization;
  canManage: boolean;
}) {
  const ui = useUiTranslations();
  const update = useUpdateOrganization();
  const stored = { ...DEFAULTS, ...(organization.self_healing ?? {}) };
  const [form, setForm] = useState<SelfHealingPolicy>(stored);
  const [savedForm, setSavedForm] = useState<SelfHealingPolicy>(stored);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const dirty = JSON.stringify(form) !== JSON.stringify(savedForm);
  const readOnly = !canManage || update.isPending;

  const set = (field: keyof SelfHealingPolicy, value: string) => {
    if (readOnly) return;
    setForm((current) => ({ ...current, [field]: value }) as SelfHealingPolicy);
    setSaved(false);
    setError(null);
  };

  async function save() {
    if (readOnly) return;
    try {
      const result = await update.mutateAsync({ self_healing: form });
      const next = { ...DEFAULTS, ...(result.self_healing ?? form) };
      setForm(next);
      setSavedForm(next);
      setSaved(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not save."));
    }
  }

  return (
    <Card className="space-y-6 p-6" data-testid="self-healing-settings">
      <div className="space-y-1">
        <h2 className="font-medium">{ui("When a finding opens")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui(
            "What Everruns does on its own before the limits above stop the work. Every action is recorded on the finding, the people the finding notifies are told, and it can be undone from the finding.",
          )}
        </p>
      </div>
      <div className="space-y-1.5">
        <Label>{ui("A schedule keeps repeating itself")}</Label>
        <div>
          <SegmentedControl
            label={ui("A schedule keeps repeating itself")}
            value={form.runaway_schedules}
            onValueChange={(value) => set("runaway_schedules", value)}
            options={[
              { value: "auto", label: ui("Pause it") },
              { value: "suggest", label: ui("Offer to pause it") },
              { value: "off", label: ui("Do nothing") },
            ]}
          />
        </div>
        <p className="text-xs text-muted-foreground">
          {ui(
            "When its runs end the same way 24 times in a row. Whatever you pick, it still stops at 48.",
          )}
        </p>
      </div>
      <div className="space-y-1.5">
        <Label>{ui("Changes to an agent's instructions")}</Label>
        <div>
          <SegmentedControl
            label={ui("Changes to an agent's instructions")}
            value={form.agent_fixes}
            onValueChange={(value) => set("agent_fixes", value)}
            options={[
              { value: "suggest", label: ui("Suggest") },
              { value: "off", label: ui("Don't suggest") },
            ]}
          />
        </div>
        <p className="text-xs text-muted-foreground">
          {ui(
            "The Inspector proposes text to add to the agent's instructions. A person checks it against the agent's evals and approves it; it is never applied on its own.",
          )}
        </p>
      </div>
      {!canManage && (
        <p className="text-sm text-muted-foreground">
          {ui("Only organization owners and admins can change these settings.")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {canManage && (
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
