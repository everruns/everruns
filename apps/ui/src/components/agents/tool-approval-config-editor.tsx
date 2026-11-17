"use client";

// Agent → Capabilities → Tool Approval. Mirrors the `tool_approval` config in
// crates/core/src/builtins/tool_approval.rs: mode, rules, reviewer notes and
// the approval window. The generic schema form showed the raw mode names and
// no way to read a rule table; this editor is the picker. Fields it does not
// own are written back unchanged.

import { useState } from "react";
import { useUiTranslations } from "@/providers/locale-provider";
import { Label } from "@/components/ui/label";
import {
  ApprovalModePicker,
  ApprovalNotesEditor,
  ApprovalRulesEditor,
  isApprovalMode,
  readNotes,
  readRules,
  savedNotes,
  savedRules,
} from "@/components/tool-approval/approval-controls";
import { useOrganization } from "@/hooks/use-organizations";
import { APPROVAL_MODES, type ApprovalModeName, type ToolApprovalRule } from "@/lib/api/types";

export const TOOL_APPROVAL_CAPABILITY_ID = "tool_approval";

const AGENT_ACTIONS = ["ask", "never", "allow"] as const;

export function ToolApprovalConfigEditor({
  config,
  onChange,
  disabled,
}: {
  config: Record<string, unknown>;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}) {
  const ui = useUiTranslations();
  const { data: organization } = useOrganization();
  const mode: ApprovalModeName = isApprovalMode(config.mode) ? config.mode : "normal";
  const auto =
    config.auto && typeof config.auto === "object" ? (config.auto as Record<string, unknown>) : {};
  // Rows being typed live here; the config only ever holds filled-in rows, so a
  // half-written rule never reaches the server's validation.
  const [rules, setRuleDrafts] = useState<ToolApprovalRule[]>(() => readRules(config.rules));
  const [notes, setNoteDrafts] = useState<string[]>(() => readNotes(auto.notes));
  const allowed = organization?.tool_approval_policy?.allowed_modes ?? [];
  // A mode the organization no longer allows stays visible while it is selected,
  // so the editor never hides what the agent is saved with.
  const modes = APPROVAL_MODES.filter(
    (candidate) => allowed.length === 0 || allowed.includes(candidate) || candidate === mode,
  );

  const write = (patch: Record<string, unknown>) => {
    const next: Record<string, unknown> = { ...config, ...patch };
    for (const [key, value] of Object.entries(patch)) {
      if (value === undefined) delete next[key];
    }
    onChange(next);
  };
  const setRules = (drafts: ToolApprovalRule[]) => {
    setRuleDrafts(drafts);
    const next = savedRules(drafts);
    write({ rules: next.length > 0 ? next : undefined });
  };
  const setNotes = (drafts: string[]) => {
    setNoteDrafts(drafts);
    const next = savedNotes(drafts);
    const rest = { ...auto };
    if (next.length > 0) rest.notes = next;
    else delete rest.notes;
    write({ auto: Object.keys(rest).length > 0 ? rest : undefined });
  };

  return (
    <div className="space-y-5 pt-2" data-testid="tool-approval-config-editor">
      <div className="space-y-1.5">
        <Label>{ui("Approval mode")}</Label>
        <ApprovalModePicker
          name="agent-approval-mode"
          label={ui("Approval mode")}
          value={mode}
          modes={modes}
          disabled={disabled}
          onChange={(next) => write({ mode: next ?? "normal" })}
        />
        {!modes.includes("auto") && (
          <p className="text-xs text-muted-foreground">
            {ui("Auto shows only when the organization allows it.")}
          </p>
        )}
      </div>

      <div className="space-y-1.5">
        <Label>{ui("Rules")}</Label>
        <p className="text-xs text-muted-foreground">
          {ui(
            "Rules apply in every mode; the strictest match wins. A tool name can use * as a wildcard.",
          )}
        </p>
        <ApprovalRulesEditor
          idPrefix="agent-approval-rule"
          rules={rules}
          actions={AGENT_ACTIONS}
          disabled={disabled}
          onChange={setRules}
        />
      </div>

      <div className="space-y-1.5">
        <Label>{ui("Notes for the reviewer")}</Label>
        <p className="text-xs text-muted-foreground">
          {ui(
            "Plain sentences the auto reviewer reads, such as which repositories are yours or what counts as production.",
          )}
        </p>
        <ApprovalNotesEditor
          notes={notes}
          disabled={disabled}
          placeholder={ui("github.com/acme is ours; pushing branches there is routine.")}
          onChange={setNotes}
        />
      </div>
    </div>
  );
}
