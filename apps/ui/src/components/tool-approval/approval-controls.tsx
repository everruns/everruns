"use client";

/**
 * Shared controls for the tool approval gate: the mode picker, the rules
 * table and the reviewer notes list. The agent's capability settings, the
 * organization policy and each person's own settings all use them, so a mode
 * or an action reads the same everywhere
 * (knowledge/execution/tool-approval.md, "Auto mode").
 *
 * Decisions:
 * - Copy is English source text passed through `ui()`; the catalogs in
 *   src/lib/messages translate it.
 * - The rules editor takes the actions a layer may use: personal rules only
 *   add caution, so they never offer Allow.
 */

import { Plus, X } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type { ApprovalModeName, ToolApprovalRule, ToolRuleActionName } from "@/lib/api/types";
import { cn } from "@/lib/utils";

export const MODE_LABELS: Record<ApprovalModeName, string> = {
  off: "Off",
  auto: "Auto",
  normal: "Normal",
  protective: "Protective",
};

export const MODE_HINTS: Record<ApprovalModeName, string> = {
  off: "Never asks",
  auto: "A reviewer answers first, asks you when unsure",
  normal: "Asks before risky calls",
  protective: "Asks before any change",
};

export const ACTION_LABELS: Record<ToolRuleActionName, string> = {
  allow: "Allow",
  ask: "Always ask",
  never: "Never",
};

/** Most rules and notes one layer may hold (crates/contracts tool_approval_policy.rs). */
export const MAX_APPROVAL_RULES = 50;
export const MAX_TRUST_NOTES = 20;

export function isApprovalMode(value: unknown): value is ApprovalModeName {
  return value === "off" || value === "auto" || value === "normal" || value === "protective";
}

function isRuleAction(value: unknown): value is ToolRuleActionName {
  return value === "allow" || value === "ask" || value === "never";
}

/** Rules from an untyped config value; malformed rows are dropped. */
export function readRules(value: unknown): ToolApprovalRule[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry) => {
    if (!entry || typeof entry !== "object") return [];
    const { tool, action } = entry as { tool?: unknown; action?: unknown };
    return typeof tool === "string" && isRuleAction(action) ? [{ tool, action }] : [];
  });
}

/** Notes from an untyped config value. */
export function readNotes(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === "string");
}

/** Rules ready to save: blank patterns are a row someone has not filled in yet. */
export function savedRules(rules: ToolApprovalRule[]): ToolApprovalRule[] {
  return rules
    .map((rule) => ({ ...rule, tool: rule.tool.trim() }))
    .filter((rule) => rule.tool.length > 0);
}

/** Notes ready to save. */
export function savedNotes(notes: string[]): string[] {
  return notes.map((note) => note.trim()).filter((note) => note.length > 0);
}

export function ApprovalModePicker({
  value,
  onChange,
  modes,
  disabled,
  name,
  label,
  extraOption,
}: {
  value: ApprovalModeName | null;
  onChange: (mode: ApprovalModeName | null) => void;
  /** Modes to offer, in ladder order. */
  modes: readonly ApprovalModeName[];
  disabled?: boolean;
  /** Radio group name, unique per page. */
  name: string;
  label: string;
  /** An option that picks no mode, such as "Agent default". */
  extraOption?: { label: string; hint: string };
}) {
  const ui = useUiTranslations();
  const options: Array<{ value: ApprovalModeName | null; label: string; hint: string }> = [
    ...(extraOption ? [{ value: null, label: extraOption.label, hint: extraOption.hint }] : []),
    ...modes.map((mode) => ({
      value: mode,
      label: ui(MODE_LABELS[mode]),
      hint: ui(MODE_HINTS[mode]),
    })),
  ];
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="grid gap-px border border-border bg-border sm:grid-cols-2"
    >
      {options.map((option) => {
        const checked = option.value === value;
        const id = `${name}-${option.value ?? "default"}`;
        return (
          <label
            key={id}
            htmlFor={id}
            className={cn(
              "flex cursor-pointer items-start gap-2.5 bg-background px-3 py-2.5",
              checked && "bg-muted",
              disabled && "cursor-not-allowed opacity-60",
            )}
          >
            <input
              id={id}
              type="radio"
              name={name}
              className="mt-1 accent-[hsl(var(--primary))]"
              checked={checked}
              disabled={disabled}
              onChange={() => onChange(option.value)}
            />
            <span className="min-w-0">
              <span className="block text-sm font-medium text-foreground">{option.label}</span>
              <span className="block text-xs text-muted-foreground">{option.hint}</span>
            </span>
          </label>
        );
      })}
    </div>
  );
}

export function ApprovalRulesEditor({
  rules,
  onChange,
  actions,
  disabled,
  idPrefix,
}: {
  rules: ToolApprovalRule[];
  onChange: (rules: ToolApprovalRule[]) => void;
  /** Actions this layer may use. */
  actions: readonly ToolRuleActionName[];
  disabled?: boolean;
  idPrefix: string;
}) {
  const ui = useUiTranslations();
  const update = (index: number, rule: ToolApprovalRule) =>
    onChange(rules.map((current, i) => (i === index ? rule : current)));
  return (
    <div className="space-y-2">
      {rules.length > 0 && (
        <div className="border border-border">
          <div className="grid grid-cols-[minmax(0,1fr)_10rem_2rem] gap-2 border-b border-border bg-muted/50 px-2 py-1.5 text-xs font-medium text-muted-foreground">
            <span>{ui("Tool")}</span>
            <span>{ui("Action")}</span>
            <span />
          </div>
          {rules.map((rule, index) => (
            <div
              key={index}
              className="grid grid-cols-[minmax(0,1fr)_10rem_2rem] items-center gap-2 border-b border-border px-2 py-1.5 last:border-b-0"
            >
              <Input
                aria-label={ui("Tool name or pattern")}
                placeholder="github_merge_*"
                className="h-8 font-mono text-xs"
                value={rule.tool}
                disabled={disabled}
                onChange={(event) => update(index, { ...rule, tool: event.target.value })}
              />
              <Select
                value={rule.action}
                disabled={disabled}
                onValueChange={(value) =>
                  update(index, { ...rule, action: value as ToolRuleActionName })
                }
              >
                <SelectTrigger
                  id={`${idPrefix}-action-${index}`}
                  aria-label={ui("Action")}
                  className="h-8 w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {actions.map((action) => (
                    <SelectItem key={action} value={action}>
                      {ui(ACTION_LABELS[action])}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="h-8 w-8"
                aria-label={ui("Remove rule for {tool}", { tool: rule.tool || "…" })}
                disabled={disabled}
                onClick={() => onChange(rules.filter((_, i) => i !== index))}
              >
                <X className="h-4 w-4" />
              </Button>
            </div>
          ))}
        </div>
      )}
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={disabled || rules.length >= MAX_APPROVAL_RULES}
        onClick={() => onChange([...rules, { tool: "", action: actions[0] }])}
      >
        <Plus className="h-4 w-4" />
        {ui("Add rule")}
      </Button>
    </div>
  );
}

export function ApprovalNotesEditor({
  notes,
  onChange,
  disabled,
  placeholder,
}: {
  notes: string[];
  onChange: (notes: string[]) => void;
  disabled?: boolean;
  placeholder: string;
}) {
  const ui = useUiTranslations();
  return (
    <div className="space-y-2">
      {notes.map((note, index) => (
        <div key={index} className="flex items-start gap-2">
          <Textarea
            aria-label={ui("Note {number}", { number: index + 1 })}
            rows={2}
            className="min-h-0 text-sm"
            placeholder={placeholder}
            value={note}
            disabled={disabled}
            onChange={(event) =>
              onChange(notes.map((current, i) => (i === index ? event.target.value : current)))
            }
          />
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0"
            aria-label={ui("Remove note {number}", { number: index + 1 })}
            disabled={disabled}
            onClick={() => onChange(notes.filter((_, i) => i !== index))}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      ))}
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={disabled || notes.length >= MAX_TRUST_NOTES}
        onClick={() => onChange([...notes, ""])}
      >
        <Plus className="h-4 w-4" />
        {ui("Add a note")}
      </Button>
    </div>
  );
}
