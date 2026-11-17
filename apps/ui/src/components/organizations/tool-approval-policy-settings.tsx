"use client";

// Settings → Approvals: the organization's tool approval policy
// (knowledge/execution/tool-approval.md, "Auto mode"). Admins edit; everyone
// else reads. PATCH replaces the whole `tool_approval_policy` object, so the
// form sends every field. An empty allowed-modes list means every mode is
// allowed; the form saves it that way when all four boxes are ticked.

import { useState } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import {
  ApprovalNotesEditor,
  ApprovalRulesEditor,
  MODE_HINTS,
  MODE_LABELS,
  savedNotes,
  savedRules,
} from "@/components/tool-approval/approval-controls";
import { useUpdateOrganization } from "@/hooks/use-organizations";
import { listAuditLogs } from "@/lib/api/audit-logs";
import {
  APPROVAL_MODES,
  AUTO_REVIEW_AUDIT_EVENT,
  type ApprovalModeName,
  type Organization,
  type OrgToolApprovalPolicy,
  type ToolApprovalRule,
} from "@/lib/api/types";
import { formatRelativeTime } from "@/lib/formatting";
import { queryKeys } from "@/lib/query-keys";

const ORG_ACTIONS = ["ask", "never", "allow"] as const;

interface PolicyForm {
  modes: ApprovalModeName[];
  rules: ToolApprovalRule[];
  notes: string[];
}

function formFrom(policy: OrgToolApprovalPolicy | undefined): PolicyForm {
  const allowed = (policy?.allowed_modes ?? []).filter((mode): mode is ApprovalModeName =>
    APPROVAL_MODES.includes(mode as ApprovalModeName),
  );
  return {
    modes: allowed.length > 0 ? allowed : [...APPROVAL_MODES],
    rules: policy?.rules ?? [],
    notes: policy?.trust_notes ?? [],
  };
}

function formKey(form: PolicyForm): string {
  const policy = policyFrom(form);
  return JSON.stringify({ ...policy, modes: [...form.modes].sort() });
}

function policyFrom(form: PolicyForm): OrgToolApprovalPolicy {
  const everyMode = APPROVAL_MODES.every((mode) => form.modes.includes(mode));
  return {
    allowed_modes: everyMode ? [] : APPROVAL_MODES.filter((mode) => form.modes.includes(mode)),
    rules: savedRules(form.rules),
    trust_notes: savedNotes(form.notes),
  };
}

export function ToolApprovalPolicySettings({
  organization,
  canManage,
}: {
  organization: Organization;
  canManage: boolean;
}) {
  const ui = useUiTranslations();
  const update = useUpdateOrganization();
  const [form, setForm] = useState<PolicyForm>(() => formFrom(organization.tool_approval_policy));
  const [saved, setSaved] = useState(() => formKey(formFrom(organization.tool_approval_policy)));
  const [error, setError] = useState<string | null>(null);
  const [justSaved, setJustSaved] = useState(false);
  // Compared on the form, not the saved policy: no mode ticked and every mode
  // ticked both save as an empty list, but only one of them may be saved.
  const dirty = formKey(form) !== saved;
  const readOnly = !canManage || update.isPending;

  const change = (patch: Partial<PolicyForm>) => {
    if (readOnly) return;
    setForm((current) => ({ ...current, ...patch }));
    setJustSaved(false);
    setError(null);
  };

  const toggleMode = (mode: ApprovalModeName, on: boolean) => {
    const modes = on ? [...form.modes, mode] : form.modes.filter((m) => m !== mode);
    change({ modes });
  };

  async function save() {
    if (readOnly) return;
    if (form.modes.length === 0) {
      setError(ui("Allow at least one mode."));
      return;
    }
    try {
      const result = await update.mutateAsync({ tool_approval_policy: policyFrom(form) });
      const next = formFrom(result.tool_approval_policy ?? policyFrom(form));
      setForm(next);
      setSaved(formKey(next));
      setJustSaved(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not save."));
    }
  }

  return (
    <div className="space-y-6">
      <Card className="space-y-6 p-6" data-testid="tool-approval-policy-settings">
        <div className="space-y-1.5">
          <h2 className="font-medium">{ui("Allowed modes")}</h2>
          <p className="text-sm text-muted-foreground">
            {ui(
              "Modes agents and people may pick. A mode you turn off is raised to the next more careful one you allow.",
            )}
          </p>
          <div className="grid gap-2 pt-1 sm:grid-cols-2">
            {APPROVAL_MODES.map((mode) => (
              <div key={mode} className="flex items-start gap-2.5">
                <Checkbox
                  id={`org-approval-mode-${mode}`}
                  className="mt-0.5"
                  checked={form.modes.includes(mode)}
                  disabled={readOnly}
                  onCheckedChange={(on) => toggleMode(mode, on)}
                />
                <Label htmlFor={`org-approval-mode-${mode}`} className="font-normal">
                  <span className="block text-sm font-medium">{ui(MODE_LABELS[mode])}</span>
                  <span className="block text-xs text-muted-foreground">
                    {ui(MODE_HINTS[mode])}
                  </span>
                </Label>
              </div>
            ))}
          </div>
        </div>

        <div className="space-y-1.5">
          <h2 className="font-medium">{ui("Rules for every agent")}</h2>
          <p className="text-sm text-muted-foreground">
            {ui(
              "These apply to every agent in the organization, in every mode. When rules disagree, the most cautious one wins.",
            )}
          </p>
          <ApprovalRulesEditor
            idPrefix="org-approval-rule"
            rules={form.rules}
            actions={ORG_ACTIONS}
            disabled={readOnly}
            onChange={(rules) => change({ rules })}
          />
        </div>

        <div className="space-y-1.5">
          <h2 className="font-medium">{ui("Trust notes for the reviewer")}</h2>
          <p className="text-sm text-muted-foreground">
            {ui(
              "Plain sentences the auto reviewer reads for every agent: your domains, your repositories, which systems are production.",
            )}
          </p>
          <ApprovalNotesEditor
            notes={form.notes}
            disabled={readOnly}
            placeholder={ui("Only the prod-* clusters are production.")}
            onChange={(notes) => change({ notes })}
          />
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
            {justSaved && !dirty && (
              <span role="status" className="text-sm text-success">
                {ui("Saved")}
              </span>
            )}
          </div>
        )}
      </Card>
      {canManage && <RecentReviews organizationId={organization.id} />}
    </div>
  );
}

const VERDICT_LABELS: Record<string, string> = {
  allowed: "Allowed",
  blocked: "Blocked",
  asked_person: "Asked a person",
};

/** The last auto reviews, from the audit log. Needs audit access, so admins only. */
function RecentReviews({ organizationId }: { organizationId: string }) {
  const ui = useUiTranslations();
  const { locale } = useLocale();
  const { data, isLoading, isError } = useQuery({
    queryKey: [...queryKeys.organizations.detail(organizationId), "auto-reviews"],
    queryFn: () => listAuditLogs(organizationId, { eventType: AUTO_REVIEW_AUDIT_EVENT, limit: 10 }),
    staleTime: 30_000,
  });
  return (
    <Card className="space-y-3 p-6" data-testid="recent-auto-reviews">
      <div className="space-y-1">
        <h2 className="font-medium">{ui("Recent reviews")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui("The last calls the auto reviewer decided on, newest first.")}
        </p>
      </div>
      {isLoading ? (
        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
      ) : isError ? (
        <p className="text-sm text-muted-foreground">{ui("Could not load recent reviews.")}</p>
      ) : !data || data.length === 0 ? (
        <p className="text-sm text-muted-foreground">{ui("No auto reviews yet.")}</p>
      ) : (
        <div className="border border-border">
          <div className="grid grid-cols-[6rem_minmax(0,10rem)_minmax(0,1fr)] gap-3 border-b border-border bg-muted/50 px-3 py-1.5 text-xs font-medium text-muted-foreground">
            <span>{ui("When")}</span>
            <span>{ui("Tool")}</span>
            <span>{ui("Verdict")}</span>
          </div>
          {data.map((entry) => {
            const meta = entry.metadata;
            const verdict = typeof meta.verdict === "string" ? meta.verdict : "";
            const rule = typeof meta.rule === "string" ? meta.rule : null;
            const reason = typeof meta.reason === "string" ? meta.reason : "";
            const sessionId = typeof meta.session_id === "string" ? meta.session_id : null;
            const label = ui(VERDICT_LABELS[verdict] ?? verdict);
            return (
              <div
                key={entry.id}
                className="grid grid-cols-[6rem_minmax(0,10rem)_minmax(0,1fr)] gap-3 border-b border-border px-3 py-2 text-sm last:border-b-0"
              >
                <span className="text-muted-foreground">
                  {sessionId?.startsWith("session_") ? (
                    <Link href={`/sessions/${sessionId}`} className="hover:text-foreground">
                      {formatRelativeTime(entry.created_at, locale)}
                    </Link>
                  ) : (
                    formatRelativeTime(entry.created_at, locale)
                  )}
                </span>
                <span className="truncate font-mono text-xs leading-5">
                  {typeof meta.tool_name === "string" ? meta.tool_name : ""}
                </span>
                <span className="min-w-0" title={reason}>
                  <span className={verdict === "blocked" ? "text-destructive" : undefined}>
                    {rule ? `${label} · ${rule}` : label}
                  </span>
                  {reason && (
                    <span className="block truncate text-xs text-muted-foreground">{reason}</span>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </Card>
  );
}
