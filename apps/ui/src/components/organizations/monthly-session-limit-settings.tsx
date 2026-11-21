"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useUpdateOrganization } from "@/hooks/use-organizations";
import { useUiTranslations } from "@/providers/locale-provider";

const MAX_MONTHLY_SESSION_LIMIT = 100_000;

export function MonthlySessionLimitSettings({
  limit,
  canManage,
}: {
  limit: number;
  canManage: boolean;
}) {
  const ui = useUiTranslations();
  const update = useUpdateOrganization();
  const [draft, setDraft] = useState<string>();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const value = draft ?? String(limit);

  async function save() {
    if (!canManage || update.isPending) return;
    const nextLimit = Number(value);
    if (!Number.isInteger(nextLimit) || nextLimit < 1 || nextLimit > MAX_MONTHLY_SESSION_LIMIT) {
      setError(ui("Enter a whole number from 1 to 100,000."));
      return;
    }
    try {
      const organization = await update.mutateAsync({ monthly_session_limit: nextLimit });
      if (organization.monthly_session_limit !== nextLimit) {
        setError(ui("Could not save the session limit."));
        return;
      }
      setDraft(undefined);
      setError(undefined);
      setSaved(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not save the session limit."));
    }
  }

  return (
    <Card className="space-y-4 p-6">
      <h2 className="font-medium">{ui("Session creation")}</h2>
      <form
        className="space-y-4"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <div className="space-y-1.5">
          <Label htmlFor="monthly-session-limit">{ui("Monthly session limit")}</Label>
          <Input
            id="monthly-session-limit"
            type="number"
            min={1}
            max={MAX_MONTHLY_SESSION_LIMIT}
            step={1}
            value={value}
            readOnly={!canManage || update.isPending}
            aria-describedby="monthly-session-limit-hint"
            className="max-w-xs"
            onChange={(event) => {
              setDraft(event.target.value);
              setError(undefined);
              setSaved(false);
            }}
          />
          <p id="monthly-session-limit-hint" className="text-sm text-muted-foreground">
            {ui(
              "The limit resets on the first day of each month at midnight UTC. Archived and deleted sessions still count for the month they were created.",
            )}
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
