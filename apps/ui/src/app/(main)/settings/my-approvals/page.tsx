"use client";

// Settings → My approvals: the mode a person's sessions start in and the
// rules they add for calls made on their behalf. Stored as the `tool_approval`
// user preference (PersonalToolApproval in crates/contracts). Personal rules
// only add caution, so there is no Allow, and Off is not offered: a person
// switches a single session off from its header when the agent allows it.

import { useState } from "react";
import { Loader2, ShieldUser } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  ApprovalModePicker,
  ApprovalRulesEditor,
  isApprovalMode,
  readRules,
  savedRules,
} from "@/components/tool-approval/approval-controls";
import { usePageTitle } from "@/hooks";
import { useOrganization } from "@/hooks/use-organizations";
import { useSetUserPreference, useUserPreference } from "@/hooks/use-user-preferences";
import {
  PERSONAL_APPROVAL_PREFERENCE_KEY,
  type ApprovalModeName,
  type PersonalToolApproval,
  type ToolApprovalRule,
} from "@/lib/api/types";

const PERSONAL_MODES: readonly ApprovalModeName[] = ["auto", "normal", "protective"];
const PERSONAL_ACTIONS = ["ask", "never"] as const;

function readPersonal(value: unknown): PersonalToolApproval {
  if (!value || typeof value !== "object") return {};
  const record = value as Record<string, unknown>;
  return {
    default_mode: isApprovalMode(record.default_mode) ? record.default_mode : undefined,
    rules: readRules(record.rules).filter((rule) => rule.action !== "allow"),
  };
}

export default function MyApprovalsSettingsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("My approvals"), ui("Settings"));
  const preference = useUserPreference(PERSONAL_APPROVAL_PREFERENCE_KEY);
  const { data: organization } = useOrganization();
  if (preference.isLoading) return <Skeleton className="h-48 w-full" />;
  if (preference.error) {
    return <p role="alert">{ui("Unable to load approval settings.")}</p>;
  }
  const allowed = organization?.tool_approval_policy?.allowed_modes ?? [];
  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <div className="flex items-center gap-3">
          <ShieldUser className="size-5" />
          <h1 className="text-xl font-semibold">{ui("My approvals")}</h1>
        </div>
        <p className="text-sm text-muted-foreground">
          {ui("How tool calls in your own sessions get approved.")}
        </p>
      </div>
      <PersonalApprovalForm initial={readPersonal(preference.data?.value)} allowedModes={allowed} />
    </div>
  );
}

function PersonalApprovalForm({
  initial,
  allowedModes,
}: {
  initial: PersonalToolApproval;
  allowedModes: string[];
}) {
  const ui = useUiTranslations();
  const save = useSetUserPreference();
  const [mode, setMode] = useState<ApprovalModeName | null>(initial.default_mode ?? null);
  const [rules, setRules] = useState<ToolApprovalRule[]>(initial.rules ?? []);
  const [saved, setSaved] = useState(() => JSON.stringify(initial));
  const [error, setError] = useState<string | null>(null);
  const [justSaved, setJustSaved] = useState(false);
  const value: PersonalToolApproval = {
    ...(mode ? { default_mode: mode } : {}),
    rules: savedRules(rules),
  };
  const dirty = JSON.stringify(readPersonal(value)) !== saved;
  const modes = PERSONAL_MODES.filter(
    (candidate) =>
      allowedModes.length === 0 || allowedModes.includes(candidate) || candidate === mode,
  );

  async function submit() {
    setError(null);
    try {
      await save.mutateAsync({ key: PERSONAL_APPROVAL_PREFERENCE_KEY, value });
      setSaved(JSON.stringify(readPersonal(value)));
      setJustSaved(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not save."));
    }
  }

  return (
    <Card className="space-y-6 p-6" data-testid="personal-approval-settings">
      <div className="space-y-1.5">
        <Label>{ui("Start my sessions in")}</Label>
        <ApprovalModePicker
          name="personal-approval-mode"
          label={ui("Start my sessions in")}
          value={mode}
          modes={modes}
          disabled={save.isPending}
          extraOption={{ label: ui("Agent default"), hint: ui("Each agent's own mode") }}
          onChange={(next) => {
            setMode(next);
            setJustSaved(false);
          }}
        />
        <p className="text-xs text-muted-foreground">
          {ui(
            "Applies to sessions you start. You can still switch one session from its header, within what the organization allows.",
          )}
        </p>
      </div>

      <div className="space-y-1.5">
        <Label>{ui("My extra rules")}</Label>
        <p className="text-xs text-muted-foreground">
          {ui(
            "Personal rules can only add caution: Always ask or Never. They apply to calls agents make on your behalf.",
          )}
        </p>
        <ApprovalRulesEditor
          idPrefix="personal-approval-rule"
          rules={rules}
          actions={PERSONAL_ACTIONS}
          disabled={save.isPending}
          onChange={(next) => {
            setRules(next);
            setJustSaved(false);
          }}
        />
      </div>

      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex items-center gap-3">
        <Button onClick={submit} disabled={save.isPending || !dirty}>
          {save.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
          {ui("Save")}
        </Button>
        {justSaved && !dirty && (
          <span role="status" className="text-sm text-success">
            {ui("Saved")}
          </span>
        )}
      </div>
    </Card>
  );
}
