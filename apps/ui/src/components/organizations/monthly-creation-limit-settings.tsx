"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useUpdateOrganization } from "@/hooks/use-organizations";
import { useUiTranslations } from "@/providers/locale-provider";

const MAX_MONTHLY_CREATION_LIMIT = 100_000;

/** Labels for one limit, already translated by the caller. */
export interface MonthlyCreationLimitCopy {
  title: string;
  label: string;
  hint: string;
  saveError: string;
}

// Decision: one form for every monthly creation quota (sessions, agents); the
// fields share bounds (1 to 100,000) and the organization update request.
export function MonthlyCreationLimitSettings({
  field,
  limit,
  canManage,
  copy,
}: {
  field: "monthly_session_limit" | "monthly_agent_limit";
  limit: number;
  canManage: boolean;
  copy: MonthlyCreationLimitCopy;
}) {
  const inputId = field.replaceAll("_", "-");
  const ui = useUiTranslations();
  const update = useUpdateOrganization();
  const [draft, setDraft] = useState<string>();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const value = draft ?? String(limit);

  async function save() {
    if (!canManage || update.isPending) return;
    const nextLimit = Number(value);
    if (!Number.isInteger(nextLimit) || nextLimit < 1 || nextLimit > MAX_MONTHLY_CREATION_LIMIT) {
      setError(ui("Enter a whole number from 1 to 100,000."));
      return;
    }
    try {
      const organization = await update.mutateAsync({ [field]: nextLimit });
      if (organization[field] !== nextLimit) {
        setError(copy.saveError);
        return;
      }
      setDraft(undefined);
      setError(undefined);
      setSaved(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : copy.saveError);
    }
  }

  return (
    <Card className="space-y-4 p-6">
      <h2 className="font-medium">{copy.title}</h2>
      <form
        className="space-y-4"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <div className="space-y-1.5">
          <Label htmlFor={inputId}>{copy.label}</Label>
          <Input
            id={inputId}
            type="number"
            min={1}
            max={MAX_MONTHLY_CREATION_LIMIT}
            step={1}
            value={value}
            readOnly={!canManage || update.isPending}
            aria-describedby={`${inputId}-hint`}
            className="max-w-xs"
            onChange={(event) => {
              setDraft(event.target.value);
              setError(undefined);
              setSaved(false);
            }}
          />
          <p id={`${inputId}-hint`} className="text-sm text-muted-foreground">
            {copy.hint}
          </p>
        </div>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        {canManage && (
          <div className="flex items-center gap-3">
            <Button type="submit" disabled={update.isPending || value === String(limit)}>
              {ui("Save")}
            </Button>
            {saved && (
              <span role="status" className="text-sm text-success">
                {ui("Saved")}
              </span>
            )}
          </div>
        )}
      </form>
    </Card>
  );
}
