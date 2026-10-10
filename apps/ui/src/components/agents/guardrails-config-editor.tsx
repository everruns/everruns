"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useGuardrailExamples } from "@/hooks/use-capabilities";
import type { GuardrailExample } from "@/lib/api/types";
import { cn } from "@/lib/utils";

// Mirrors GuardrailsConfig in crates/core/src/guardrail_checks.rs. The settings
// panel used to dump that schema into a generic form, which showed the raw
// mode value and no way to pick a check. This editor is the picker.

const MAX_CHECKS = 64;
const MAX_ID_LEN = 64;

export type GuardrailMode = "active" | "advisory";
export type GuardrailStage = "output" | "tool_use" | "tool_output";
export type GuardrailOnFail = "block" | "log";
export type GuardrailEngine = "utility_llm" | "jev";
export type GuardrailCheckType =
  | "regex"
  | "blocklist"
  | "tool_pattern"
  | "llm_judge"
  | "mcp"
  | "moderation";

const CHECK_TYPES: GuardrailCheckType[] = [
  "regex",
  "blocklist",
  "tool_pattern",
  "llm_judge",
  "mcp",
  "moderation",
];

const STAGES: Record<GuardrailCheckType, GuardrailStage[]> = {
  regex: ["output", "tool_use", "tool_output"],
  blocklist: ["output", "tool_use", "tool_output"],
  tool_pattern: ["tool_use"],
  llm_judge: ["tool_use", "tool_output"],
  mcp: ["tool_use", "tool_output"],
  moderation: ["output"],
};

const MODE_LABELS: Record<GuardrailMode, string> = {
  active: "Active",
  advisory: "Advisory",
};

const STAGE_LABELS: Record<GuardrailStage, string> = {
  output: "Model output",
  tool_use: "Tool call",
  tool_output: "Tool result",
};

const TYPE_LABELS: Record<GuardrailCheckType, string> = {
  regex: "Regex",
  blocklist: "Blocklist",
  tool_pattern: "Tool name",
  llm_judge: "Policy judge",
  mcp: "MCP guardrail",
  moderation: "Moderation",
};

const TYPE_HINTS: Record<GuardrailCheckType, string> = {
  regex: "Match patterns in the stage text.",
  blocklist: "Match words or phrases.",
  tool_pattern: "Match tool names. * is a wildcard.",
  llm_judge: "A natural-language policy, answered by the utility model or the Decision API.",
  mcp: "Delegate the decision to an MCP guardrail scoped to this session.",
  moderation: "Score the finished message against safety categories.",
};

const ON_FAIL_LABELS: Record<GuardrailOnFail, string> = {
  block: "Block",
  log: "Log",
};

const ENGINE_LABELS: Record<GuardrailEngine, string> = {
  utility_llm: "Utility model",
  jev: "Decision API",
};

export interface GuardrailCheckDraft {
  key: string;
  id: string;
  stage: GuardrailStage;
  type: GuardrailCheckType;
  onFail: GuardrailOnFail;
  replacement: string;
  patterns: string;
  words: string;
  caseSensitive: boolean;
  tools: string;
  prompt: string;
  server: string;
  mcpTool: string;
  engine: GuardrailEngine;
  categories: string;
  threshold: string;
  /** Fields this editor does not own, written back unchanged. */
  extra?: Record<string, unknown>;
  /** A check whose type this editor does not understand. */
  raw?: Record<string, unknown>;
}

export interface GuardrailsDraft {
  mode: GuardrailMode;
  checks: GuardrailCheckDraft[];
}

const KNOWN_FIELDS = new Set([
  "id",
  "stage",
  "type",
  "on_fail",
  "replacement",
  "patterns",
  "words",
  "case_sensitive",
  "tools",
  "prompt",
  "server",
  "tool",
  "engine",
  "categories",
  "threshold",
]);

let keySeq = 0;
function nextKey(): string {
  keySeq += 1;
  return `guardrail-check-${keySeq}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function asLines(value: unknown): string {
  if (!Array.isArray(value)) return "";
  return value.filter((entry): entry is string => typeof entry === "string").join("\n");
}

function lines(value: string): string[] {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function isStage(value: unknown): value is GuardrailStage {
  return value === "output" || value === "tool_use" || value === "tool_output";
}

function isCheckType(value: unknown): value is GuardrailCheckType {
  return CHECK_TYPES.some((type) => type === value);
}

function isEngine(value: unknown): value is GuardrailEngine {
  return value === "utility_llm" || value === "jev";
}

export function blankCheck(type: GuardrailCheckType): GuardrailCheckDraft {
  return {
    key: nextKey(),
    id: "",
    stage: STAGES[type][0],
    type,
    onFail: "block",
    replacement: "",
    patterns: "",
    words: "",
    caseSensitive: false,
    tools: "",
    prompt: "",
    server: "",
    mcpTool: "",
    engine: "utility_llm",
    categories: "",
    threshold: "",
  };
}

function parseCheck(value: unknown): GuardrailCheckDraft {
  if (!isRecord(value) || !isCheckType(value.type)) {
    return { ...blankCheck("regex"), raw: isRecord(value) ? value : { value } };
  }
  const type = value.type;
  const extra: Record<string, unknown> = {};
  for (const [key, field] of Object.entries(value)) {
    if (!KNOWN_FIELDS.has(key)) extra[key] = field;
  }
  const stage = isStage(value.stage) ? value.stage : STAGES[type][0];
  return {
    key: nextKey(),
    id: asString(value.id),
    stage,
    type,
    onFail: value.on_fail === "log" ? "log" : "block",
    replacement: asString(value.replacement),
    patterns: asLines(value.patterns),
    words: asLines(value.words),
    caseSensitive: value.case_sensitive === true,
    tools: asLines(value.tools),
    prompt: asString(value.prompt),
    server: asString(value.server),
    mcpTool: asString(value.tool),
    engine: isEngine(value.engine) ? value.engine : "utility_llm",
    categories: asLines(value.categories),
    threshold: typeof value.threshold === "number" ? String(value.threshold) : "",
    extra: Object.keys(extra).length > 0 ? extra : undefined,
  };
}

export function parseGuardrailsDraft(config: unknown): GuardrailsDraft {
  const record = isRecord(config) ? config : {};
  const checks = Array.isArray(record.checks) ? record.checks.map(parseCheck) : [];
  return {
    mode: record.mode === "advisory" ? "advisory" : "active",
    checks,
  };
}

function parseThreshold(value: string): number | undefined {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return undefined;
  return Number(trimmed);
}

function serializeCheck(check: GuardrailCheckDraft): Record<string, unknown> {
  if (check.raw) return check.raw;
  const out: Record<string, unknown> = {
    ...(check.extra ?? {}),
    stage: check.stage,
    type: check.type,
  };
  const id = check.id.trim();
  if (id) out.id = id;
  if (check.onFail === "log") out.on_fail = "log";
  const replacement = check.replacement.trim();
  if (replacement) out.replacement = replacement;

  switch (check.type) {
    case "regex":
      out.patterns = lines(check.patterns);
      break;
    case "blocklist":
      out.words = lines(check.words);
      if (check.caseSensitive) out.case_sensitive = true;
      break;
    case "tool_pattern":
      out.tools = lines(check.tools);
      break;
    case "llm_judge":
      out.prompt = check.prompt;
      if (check.engine === "jev") {
        out.engine = "jev";
        const threshold = parseThreshold(check.threshold);
        if (threshold !== undefined) out.threshold = threshold;
      }
      break;
    case "mcp":
      out.server = check.server.trim();
      out.tool = check.mcpTool.trim();
      break;
    case "moderation": {
      const categories = lines(check.categories);
      if (categories.length > 0) out.categories = categories;
      if (check.engine === "jev") out.engine = "jev";
      const threshold = parseThreshold(check.threshold);
      if (threshold !== undefined) out.threshold = threshold;
      break;
    }
  }
  return out;
}

export function serializeGuardrailsDraft(draft: GuardrailsDraft): Record<string, unknown> {
  const config: Record<string, unknown> = {};
  if (draft.mode === "advisory") config.mode = "advisory";
  if (draft.checks.length > 0) {
    config.checks = draft.checks.map(serializeCheck);
  }
  return config;
}

export function appendPresetChecks(
  draft: GuardrailsDraft,
  presetConfig: Record<string, unknown>,
): GuardrailsDraft {
  const incoming = parseGuardrailsDraft(presetConfig).checks;
  const seen = new Set(draft.checks.map((check) => check.id.trim()).filter(Boolean));
  const added = incoming.filter((check) => {
    const id = check.id.trim();
    if (id && seen.has(id)) return false;
    if (id) seen.add(id);
    return true;
  });
  if (added.length === 0) return draft;
  return { ...draft, checks: [...draft.checks, ...added] };
}

export function guardrailCheckProblems(check: GuardrailCheckDraft): string[] {
  if (check.raw) return [];
  const problems: string[] = [];
  const id = check.id.trim();
  if ([...id].length > MAX_ID_LEN) {
    problems.push(`Id must be ${MAX_ID_LEN} characters or fewer.`);
  }
  if (!STAGES[check.type].includes(check.stage)) {
    problems.push("This check cannot run at that stage.");
  }
  const entries = (value: string) => lines(value);
  const requireEntries = (value: string, label: string) => {
    const parsed = entries(value);
    if (parsed.length === 0) problems.push(`Add at least one ${label}.`);
    if (parsed.length > 64) problems.push("At most 64 entries.");
  };
  switch (check.type) {
    case "regex":
      requireEntries(check.patterns, "pattern");
      break;
    case "blocklist":
      requireEntries(check.words, "word or phrase");
      break;
    case "tool_pattern":
      requireEntries(check.tools, "tool name");
      break;
    case "llm_judge":
      if (!check.prompt.trim()) problems.push("Write the policy this check enforces.");
      break;
    case "mcp":
      if (!check.server.trim() || !check.mcpTool.trim()) {
        problems.push("Server and tool are required.");
      }
      break;
    case "moderation":
      if (entries(check.categories).length > 64) problems.push("At most 64 categories.");
      break;
  }
  const usesThreshold =
    check.type === "moderation" || (check.type === "llm_judge" && check.engine === "jev");
  if (usesThreshold && check.threshold.trim()) {
    const threshold = parseThreshold(check.threshold);
    if (threshold === undefined || threshold > 100) {
      problems.push("Threshold must be a whole number from 0 to 100.");
    }
  }
  return problems;
}

interface GuardrailsConfigEditorProps {
  config: unknown;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

export function GuardrailsConfigEditor({
  config,
  onChange,
  disabled,
}: GuardrailsConfigEditorProps) {
  const examples = useGuardrailExamples();
  const written = useRef(config);
  const [draft, setDraft] = useState(() => parseGuardrailsDraft(config));

  useEffect(() => {
    if (config === written.current) return;
    written.current = config;
    setDraft(parseGuardrailsDraft(config));
  }, [config]);

  const commit = (next: GuardrailsDraft) => {
    setDraft(next);
    const value = serializeGuardrailsDraft(next);
    if (JSON.stringify(value) === JSON.stringify(written.current)) return;
    written.current = value;
    onChange(value);
  };

  const atCap = draft.checks.length >= MAX_CHECKS;
  const presets = examples.data ?? [];

  return (
    <div className="space-y-3 pt-2">
      <Field label="Mode" htmlFor="guardrails-mode">
        <Select
          value={draft.mode}
          onValueChange={(value) => commit({ ...draft, mode: value as GuardrailMode })}
          disabled={disabled}
        >
          <SelectTrigger id="guardrails-mode" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="active">{MODE_LABELS.active}</SelectItem>
            <SelectItem value="advisory">{MODE_LABELS.advisory}</SelectItem>
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          Advisory runs all checks but only logs hits. Use it to tune checks against false positives
          before enforcing.
        </p>
      </Field>

      <div className="space-y-1.5">
        <Label className="text-xs font-normal text-muted-foreground">Checks</Label>
        {draft.checks.length === 0 && (
          <p className="text-xs text-muted-foreground">
            Nothing is checked yet. Add a preset, or add a check and choose what it looks at.
          </p>
        )}
        {draft.checks.map((check, index) => (
          <CheckCard
            key={check.key}
            check={check}
            disabled={disabled}
            onChange={(next) =>
              commit({
                ...draft,
                checks: draft.checks.map((item, itemIndex) => (itemIndex === index ? next : item)),
              })
            }
            onRemove={() =>
              commit({
                ...draft,
                checks: draft.checks.filter((_, itemIndex) => itemIndex !== index),
              })
            }
          />
        ))}
      </div>

      <div className="flex flex-wrap gap-2">
        <DropdownMenu>
          <DropdownMenuTrigger
            type="button"
            disabled={disabled || atCap || examples.isLoading}
            className={cn(buttonVariants({ variant: "outline", size: "sm" }))}
          >
            <Plus className="size-3.5" />
            Add preset
          </DropdownMenuTrigger>
          <DropdownMenuPositioner align="start">
            <DropdownMenuContent className="w-80">
              {presets.length === 0 ? (
                <DropdownMenuItem disabled>No presets</DropdownMenuItem>
              ) : (
                presets.map((preset) => (
                  <DropdownMenuItem
                    key={preset.name}
                    className="flex-col items-start gap-0.5 whitespace-normal"
                    onClick={() => commit(appendPresetChecks(draft, preset.config))}
                  >
                    <PresetLabel preset={preset} />
                  </DropdownMenuItem>
                ))
              )}
            </DropdownMenuContent>
          </DropdownMenuPositioner>
        </DropdownMenu>
        <DropdownMenu>
          <DropdownMenuTrigger
            type="button"
            disabled={disabled || atCap}
            className={cn(buttonVariants({ variant: "outline", size: "sm" }))}
          >
            <Plus className="size-3.5" />
            Add check
          </DropdownMenuTrigger>
          <DropdownMenuPositioner align="start">
            <DropdownMenuContent className="w-72">
              {CHECK_TYPES.map((type) => (
                <DropdownMenuItem
                  key={type}
                  className="flex-col items-start gap-0.5 whitespace-normal"
                  onClick={() => commit({ ...draft, checks: [...draft.checks, blankCheck(type)] })}
                >
                  <span>{TYPE_LABELS[type]}</span>
                  <span className="text-xs text-muted-foreground">{TYPE_HINTS[type]}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenuPositioner>
        </DropdownMenu>
      </div>
      {examples.error && <p className="text-xs text-destructive">Presets could not be loaded.</p>}
    </div>
  );
}

function PresetLabel({ preset }: { preset: GuardrailExample }) {
  return (
    <>
      <span>{preset.display_name}</span>
      <span className="text-xs text-muted-foreground line-clamp-3">{preset.description}</span>
      {preset.data_egress === "utility_llm" && (
        <span className="text-xs text-muted-foreground">
          Sends an excerpt to the utility model.
        </span>
      )}
    </>
  );
}

function CheckCard({
  check,
  disabled,
  onChange,
  onRemove,
}: {
  check: GuardrailCheckDraft;
  disabled?: boolean;
  onChange: (check: GuardrailCheckDraft) => void;
  onRemove: () => void;
}) {
  const problems = guardrailCheckProblems(check);
  const patch = (next: Partial<GuardrailCheckDraft>) => onChange({ ...check, ...next });
  const changeType = (type: GuardrailCheckType) => {
    const stage = STAGES[type].includes(check.stage) ? check.stage : STAGES[type][0];
    onChange({ ...check, type, stage });
  };

  return (
    <div className="space-y-2 rounded-md border border-border/60 p-2" data-testid="guardrail-check">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          {check.raw ? (
            <p className="text-xs text-muted-foreground">
              This check uses a shape the editor does not recognize. Removing it is the only edit
              that keeps the rest of the config.
            </p>
          ) : (
            <Field label="Type" htmlFor={`${check.key}-type`}>
              <LabeledOptions
                id={`${check.key}-type`}
                value={check.type}
                disabled={disabled}
                onValueChange={(value) => changeType(value as GuardrailCheckType)}
                options={CHECK_TYPES.map((type) => ({ value: type, label: TYPE_LABELS[type] }))}
              />
            </Field>
          )}
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Remove check"
          disabled={disabled}
          className={cn("text-muted-foreground hover:text-destructive", !check.raw && "mt-5")}
          onClick={onRemove}
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>

      {!check.raw && (
        <>
          <p className="text-xs text-muted-foreground">{TYPE_HINTS[check.type]}</p>
          <Field label="Stage" htmlFor={`${check.key}-stage`}>
            <LabeledOptions
              id={`${check.key}-stage`}
              value={check.stage}
              disabled={disabled}
              onValueChange={(value) => patch({ stage: value as GuardrailStage })}
              options={(STAGES[check.type].includes(check.stage)
                ? STAGES[check.type]
                : [check.stage, ...STAGES[check.type]]
              ).map((stage) => ({
                value: stage,
                label: STAGE_LABELS[stage],
              }))}
            />
          </Field>
          <Field label="When it matches" htmlFor={`${check.key}-on-fail`}>
            <LabeledOptions
              id={`${check.key}-on-fail`}
              value={check.onFail}
              disabled={disabled}
              onValueChange={(value) => patch({ onFail: value as GuardrailOnFail })}
              options={(["block", "log"] as const).map((action) => ({
                value: action,
                label: ON_FAIL_LABELS[action],
              }))}
            />
          </Field>
          <TypeFields check={check} disabled={disabled} onChange={patch} />
          <Field label="Id" htmlFor={`${check.key}-id`}>
            <Input
              id={`${check.key}-id`}
              value={check.id}
              placeholder="Shown in logs"
              disabled={disabled}
              className="h-8 text-sm"
              onChange={(event) => patch({ id: event.target.value })}
            />
          </Field>
          <Field label="Message when blocked" htmlFor={`${check.key}-replacement`}>
            <Input
              id={`${check.key}-replacement`}
              value={check.replacement}
              disabled={disabled}
              className="h-8 text-sm"
              onChange={(event) => patch({ replacement: event.target.value })}
            />
          </Field>
          {problems.map((problem) => (
            <p key={problem} className="text-xs text-destructive">
              {problem}
            </p>
          ))}
        </>
      )}
    </div>
  );
}

function TypeFields({
  check,
  disabled,
  onChange,
}: {
  check: GuardrailCheckDraft;
  disabled?: boolean;
  onChange: (next: Partial<GuardrailCheckDraft>) => void;
}) {
  switch (check.type) {
    case "regex":
      return (
        <LineField
          id={`${check.key}-patterns`}
          label="Patterns"
          value={check.patterns}
          disabled={disabled}
          placeholder="One pattern per line"
          onChange={(patterns) => onChange({ patterns })}
        />
      );
    case "blocklist":
      return (
        <>
          <LineField
            id={`${check.key}-words`}
            label="Words"
            value={check.words}
            disabled={disabled}
            placeholder="One word or phrase per line"
            onChange={(words) => onChange({ words })}
          />
          <div className="flex items-center gap-2">
            <Checkbox
              id={`${check.key}-case`}
              checked={check.caseSensitive}
              disabled={disabled}
              onCheckedChange={(caseSensitive) => onChange({ caseSensitive })}
            />
            <Label htmlFor={`${check.key}-case`} className="text-xs font-normal">
              Case sensitive
            </Label>
          </div>
        </>
      );
    case "tool_pattern":
      return (
        <LineField
          id={`${check.key}-tools`}
          label="Tool names"
          value={check.tools}
          disabled={disabled}
          placeholder={"bash*\n*exec*"}
          onChange={(tools) => onChange({ tools })}
        />
      );
    case "llm_judge":
      return (
        <>
          <Field label="Policy" htmlFor={`${check.key}-prompt`}>
            <Textarea
              id={`${check.key}-prompt`}
              value={check.prompt}
              disabled={disabled}
              placeholder="Block any tool call that deletes customer records."
              className="min-h-16 text-[13px]"
              onChange={(event) => onChange({ prompt: event.target.value })}
            />
          </Field>
          <EngineField check={check} disabled={disabled} onChange={onChange} />
        </>
      );
    case "mcp":
      return (
        <>
          <Field label="Server" htmlFor={`${check.key}-server`}>
            <Input
              id={`${check.key}-server`}
              value={check.server}
              disabled={disabled}
              className="h-8 text-sm"
              onChange={(event) => onChange({ server: event.target.value })}
            />
          </Field>
          <Field label="Tool" htmlFor={`${check.key}-tool`}>
            <Input
              id={`${check.key}-tool`}
              value={check.mcpTool}
              disabled={disabled}
              className="h-8 text-sm"
              onChange={(event) => onChange({ mcpTool: event.target.value })}
            />
          </Field>
        </>
      );
    case "moderation":
      return (
        <>
          <LineField
            id={`${check.key}-categories`}
            label="Categories"
            hint="Leave empty for hate, harassment, self-harm, sexual, violence, and illicit."
            value={check.categories}
            disabled={disabled}
            placeholder="One category per line"
            onChange={(categories) => onChange({ categories })}
          />
          <EngineField check={check} disabled={disabled} onChange={onChange} />
        </>
      );
  }
}

function EngineField({
  check,
  disabled,
  onChange,
}: {
  check: GuardrailCheckDraft;
  disabled?: boolean;
  onChange: (next: Partial<GuardrailCheckDraft>) => void;
}) {
  const showThreshold = check.type === "moderation" || check.engine === "jev";
  return (
    <>
      <Field label="Answered by" htmlFor={`${check.key}-engine`}>
        <LabeledOptions
          id={`${check.key}-engine`}
          value={check.engine}
          disabled={disabled}
          onValueChange={(value) => onChange({ engine: value as GuardrailEngine })}
          options={(["utility_llm", "jev"] as const).map((engine) => ({
            value: engine,
            label: ENGINE_LABELS[engine],
          }))}
        />
        <p className="text-xs text-muted-foreground">
          {check.engine === "jev"
            ? "Jev returns a probability. The check trips at or above the threshold."
            : "The utility model returns allow or block."}
        </p>
      </Field>
      {showThreshold && (
        <Field label="Threshold" htmlFor={`${check.key}-threshold`}>
          <Input
            id={`${check.key}-threshold`}
            value={check.threshold}
            placeholder="50"
            inputMode="numeric"
            disabled={disabled}
            className="h-8 text-sm"
            onChange={(event) => onChange({ threshold: event.target.value })}
          />
        </Field>
      )}
    </>
  );
}

function LineField({
  id,
  label,
  hint,
  value,
  placeholder,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  hint?: string;
  value: string;
  placeholder?: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <Field label={label} htmlFor={id} hint={hint}>
      <Textarea
        id={id}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        className="min-h-16 font-mono text-[13px]"
        onChange={(event) => onChange(event.target.value)}
      />
    </Field>
  );
}

function Field({
  label,
  htmlFor,
  hint,
  children,
}: {
  label: string;
  htmlFor?: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="space-y-1">
      <Label htmlFor={htmlFor} className="text-xs font-normal text-muted-foreground">
        {label}
      </Label>
      {children}
      {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
    </div>
  );
}

function LabeledOptions({
  id,
  value,
  options,
  disabled,
  onValueChange,
}: {
  id: string;
  value: string;
  options: { value: string; label: string }[];
  disabled?: boolean;
  onValueChange: (value: string) => void;
}) {
  return (
    <Select value={value} onValueChange={onValueChange} disabled={disabled}>
      <SelectTrigger id={id} className="w-full">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
