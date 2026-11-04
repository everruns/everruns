"use client";

// Scorers editor for the observer form.
//
// Scope is organized as tabs labelled "Answers" (turn), "Conversations" (session),
// and "Tools" (tool). Each tab lists and adds the scorers of its scope. A
// conversation scorer grades the whole session once it has been idle a while;
// a tool scorer grades single calls, optionally only to the tools it names.

import { useUiTranslations } from "@/providers/locale-provider";
import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { ModelPicker } from "@/components/models/model-picker";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { SCORER_CATALOG } from "./scorer-catalog";
import type { ObserverScorerConfig, ObserverScorerScope, ScorerRule } from "@/lib/api/types";

// Rule variants offered for observers (file_contains excluded — server rejects it).
const RULE_TYPES = [
  "contains",
  "not_contains",
  "regex",
  "tool_called",
  "tool_not_called",
  "tool_call_count",
  "turns_within",
  "json_schema",
] as const;

type RuleType = (typeof RULE_TYPES)[number];

const RULE_LABELS: Record<RuleType, string> = {
  contains: "Contains text",
  not_contains: "Does not contain text",
  regex: "Matches regex",
  tool_called: "Tool called",
  tool_not_called: "Tool not called",
  tool_call_count: "Tool call count",
  turns_within: "Turns within",
  json_schema: "Matches JSON schema",
};

/** Human-readable one-line summary of a rule, for read-only config display. */
export function describeRule(rule: ScorerRule): string {
  switch (rule.type) {
    case "contains":
      return `contains("${rule.text}")`;
    case "not_contains":
      return `not_contains("${rule.text}")`;
    case "regex":
      return `regex(/${rule.pattern}/)`;
    case "tool_called":
      return `tool_called("${rule.tool}", min ${rule.min ?? 1})`;
    case "tool_not_called":
      return `tool_not_called("${rule.tool}")`;
    case "tool_call_count":
      return `tool_call_count(${rule.min ?? 0}–${rule.max ?? "∞"})`;
    case "turns_within":
      return `turns_within(${rule.max})`;
    case "json_schema":
      return "json_schema(…)";
  }
}

function defaultRule(type: RuleType): ScorerRule {
  switch (type) {
    case "contains":
      return { type, text: "" };
    case "not_contains":
      return { type, text: "" };
    case "regex":
      return { type, pattern: "" };
    case "tool_called":
      return { type, tool: "", min: 1 };
    case "tool_not_called":
      return { type, tool: "" };
    case "tool_call_count":
      return { type, min: 0 };
    case "turns_within":
      return { type, max: 1 };
    case "json_schema":
      return { type, schema: {} };
  }
}

function defaultScorer(scope: ObserverScorerScope): ObserverScorerConfig {
  return {
    key: "",
    scope,
    method: "rule",
    rule: defaultRule(scope === "tool" ? "not_contains" : "contains"),
  };
}

const SCOPES: ObserverScorerScope[] = ["turn", "session", "tool"];

interface ScorerEditorProps {
  scorers: ObserverScorerConfig[];
  onChange: (scorers: ObserverScorerConfig[]) => void;
  errors?: Record<number, string>;
}

export function ScorerEditor({ scorers, onChange, errors }: ScorerEditorProps) {
  const ui = useUiTranslations();
  const [scopeTab, setScopeTab] = useState<ObserverScorerScope>("turn");

  const updateScorer = (index: number, next: ObserverScorerConfig) => {
    onChange(scorers.map((s, i) => (i === index ? next : s)));
  };

  const removeScorer = (index: number) => {
    onChange(scorers.filter((_, i) => i !== index));
  };

  const addScorer = (scope: ObserverScorerScope) => {
    onChange([...scorers, defaultScorer(scope)]);
  };

  const addFromTemplate = (templateKey: string) => {
    const template = SCORER_CATALOG.find((t) => t.key === templateKey);
    if (!template) return;
    const next: ObserverScorerConfig = {
      key: template.key,
      scope: "turn",
      method: "llm_judge",
      rubric: template.rubric,
      pass_threshold: template.pass_threshold,
    };
    onChange([...scorers, next]);
  };

  const scopeLabel: Record<ObserverScorerScope, string> = {
    turn: ui("Answers"),
    session: ui("Conversations"),
    tool: ui("Tools"),
  };
  const scopeHint: Record<ObserverScorerScope, string> = {
    turn: ui("Grades each answer the agent gives."),
    session: ui(
      "Grades the whole conversation, once the session has been idle for 10 minutes. Judges see the full transcript.",
    ),
    tool: ui(
      "Grades single tool calls: the call's arguments as the input and its result as the answer.",
    ),
  };

  return (
    <div className="space-y-4">
      <Tabs value={scopeTab} onValueChange={(v) => setScopeTab(v as ObserverScorerScope)}>
        <TabsList>
          {SCOPES.map((scope) => {
            const count = scorers.filter((s) => (s.scope ?? "turn") === scope).length;
            return (
              <TabsTrigger key={scope} value={scope}>
                {scopeLabel[scope]}
                {count > 0 ? <span className="ml-1.5 tabular-nums">{count}</span> : null}
              </TabsTrigger>
            );
          })}
        </TabsList>

        {SCOPES.map((scope) => {
          const entries = scorers
            .map((scorer, index) => ({ scorer, index }))
            .filter(({ scorer }) => (scorer.scope ?? "turn") === scope);
          return (
            <TabsContent key={scope} value={scope} className="space-y-4">
              <p className="text-sm text-muted-foreground">{scopeHint[scope]}</p>
              {scope === "turn" && (
                <div className="space-y-2">
                  <Label className="text-xs text-muted-foreground">{ui("Starter templates")}</Label>
                  <div className="flex flex-wrap gap-2">
                    {SCORER_CATALOG.map((template) => (
                      <Button
                        key={template.key}
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => addFromTemplate(template.key)}
                        title={template.description}
                      >
                        <Plus className="size-3.5" />
                        {template.label}
                      </Button>
                    ))}
                  </div>
                </div>
              )}

              {entries.length === 0 ? (
                <div className="border border-dashed p-6 text-center text-sm text-muted-foreground">
                  {scope === "turn"
                    ? ui("No scorers yet. Add one from a template above or start from scratch.")
                    : ui("No scorers for this scope yet.")}
                </div>
              ) : (
                <div className="space-y-4">
                  {entries.map(({ scorer, index }) => (
                    <ScorerRow
                      key={index}
                      scorer={scorer}
                      index={index}
                      error={errors?.[index]}
                      onChange={(next) => updateScorer(index, next)}
                      onRemove={() => removeScorer(index)}
                    />
                  ))}
                </div>
              )}

              <Button type="button" variant="outline" size="sm" onClick={() => addScorer(scope)}>
                <Plus className="size-4" />
                {ui("Add scorer")}
              </Button>
            </TabsContent>
          );
        })}
      </Tabs>
    </div>
  );
}

interface ScorerRowProps {
  scorer: ObserverScorerConfig;
  index: number;
  error?: string;
  onChange: (next: ObserverScorerConfig) => void;
  onRemove: () => void;
}

function ScorerRow({ scorer, index, error, onChange, onRemove }: ScorerRowProps) {
  const ui = useUiTranslations();
  const setMethod = (method: "rule" | "llm_judge") => {
    if (method === scorer.method) return;
    if (method === "rule") {
      onChange({
        key: scorer.key,
        scope: scorer.scope,
        tools: scorer.tools,
        method: "rule",
        rule: defaultRule("contains"),
      });
    } else {
      onChange({
        key: scorer.key,
        scope: scorer.scope,
        tools: scorer.tools,
        method: "llm_judge",
        rubric: "",
        pass_threshold: 0.5,
      });
    }
  };

  return (
    <div className="space-y-4 border p-4">
      <div className="flex items-start gap-3">
        <div className="flex-1 space-y-2">
          <Label htmlFor={`scorer-key-${index}`}>{ui("Key")}</Label>
          <Input
            id={`scorer-key-${index}`}
            placeholder={ui("e.g. answer_completeness")}
            value={scorer.key}
            onChange={(e) => onChange({ ...scorer, key: e.target.value })}
          />
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="mt-7"
          onClick={onRemove}
          aria-label={ui("Remove scorer")}
        >
          <Trash2 className="size-4 text-muted-foreground" />
        </Button>
      </div>

      {scorer.scope === "tool" && (
        <ToolsField
          index={index}
          tools={scorer.tools}
          onChange={(tools) => onChange({ ...scorer, tools })}
        />
      )}

      <div className="space-y-2">
        <Label>{ui("Method")}</Label>
        <div className="flex gap-2">
          {(["rule", "llm_judge"] as const).map((m) => (
            <Button
              key={m}
              type="button"
              variant={scorer.method === m ? "default" : "outline"}
              size="sm"
              onClick={() => setMethod(m)}
            >
              {m === "rule" ? ui("Rule") : ui("LLM judge")}
            </Button>
          ))}
        </div>
      </div>

      {scorer.method === "llm_judge" ? (
        <LlmJudgeFields scorer={scorer} index={index} onChange={onChange} />
      ) : (
        <RuleFields scorer={scorer} index={index} onChange={onChange} />
      )}

      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}

/** Tool names as typed text, so commas survive while typing. */
function ToolsField({
  index,
  tools,
  onChange,
}: {
  index: number;
  tools?: string[];
  onChange: (tools: string[] | undefined) => void;
}) {
  const ui = useUiTranslations();
  const [text, setText] = useState((tools ?? []).join(", "));
  return (
    <div className="space-y-2">
      <Label htmlFor={`scorer-tools-${index}`}>{ui("Tools (optional)")}</Label>
      <Input
        id={`scorer-tools-${index}`}
        placeholder={ui("e.g. create_ticket, refund_order")}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          const parsed = e.target.value
            .split(",")
            .map((t) => t.trim())
            .filter(Boolean);
          onChange(parsed.length > 0 ? parsed : undefined);
        }}
      />
      <p className="text-xs text-muted-foreground">
        {ui("Comma-separated. Leave empty to grade every tool call.")}
      </p>
    </div>
  );
}

function LlmJudgeFields({
  scorer,
  index,
  onChange,
}: {
  scorer: Extract<ObserverScorerConfig, { method: "llm_judge" }>;
  index: number;
  onChange: (next: ObserverScorerConfig) => void;
}) {
  const ui = useUiTranslations();
  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor={`scorer-rubric-${index}`}>{ui("Rubric")}</Label>
        <Textarea
          id={`scorer-rubric-${index}`}
          placeholder={ui("Describe how the judge should score this turn...")}
          value={scorer.rubric}
          onChange={(e) => onChange({ ...scorer, rubric: e.target.value })}
          rows={3}
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor={`scorer-model-${index}`}>
          {ui("Judge model (defaults to your org default model when unset)")}
        </Label>
        <ModelPicker
          id={`scorer-model-${index}`}
          value={scorer.model_id ?? ""}
          onChange={(modelId) => onChange({ ...scorer, model_id: modelId || undefined })}
          placeholder={ui("Use org default model")}
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor={`scorer-threshold-${index}`}>{ui("Pass threshold (0–1)")}</Label>
        <Input
          id={`scorer-threshold-${index}`}
          type="number"
          min={0}
          max={1}
          step={0.05}
          value={scorer.pass_threshold}
          onChange={(e) => onChange({ ...scorer, pass_threshold: Number(e.target.value) })}
        />
      </div>
    </div>
  );
}

function RuleFields({
  scorer,
  index,
  onChange,
}: {
  scorer: Extract<ObserverScorerConfig, { method: "rule" }>;
  index: number;
  onChange: (next: ObserverScorerConfig) => void;
}) {
  const ui = useUiTranslations();
  const rule = scorer.rule;

  const setRule = (next: ScorerRule) => onChange({ ...scorer, rule: next });

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor={`scorer-rule-type-${index}`}>{ui("Rule type")}</Label>
        <Select
          value={rule.type}
          onValueChange={(value) => setRule(defaultRule(value as RuleType))}
        >
          <SelectTrigger id={`scorer-rule-type-${index}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {RULE_TYPES.map((type) => (
              <SelectItem key={type} value={type}>
                {RULE_LABELS[type]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <RuleParamFields rule={rule} index={index} onChange={setRule} />

      <div className="space-y-2">
        <Label htmlFor={`scorer-weight-${index}`}>{ui("Weight (optional, default 1.0)")}</Label>
        <Input
          id={`scorer-weight-${index}`}
          type="number"
          min={0}
          step={0.1}
          value={rule.weight ?? ""}
          placeholder="1.0"
          onChange={(e) => {
            const raw = e.target.value;
            setRule({
              ...rule,
              weight: raw === "" ? undefined : Number(raw),
            } as ScorerRule);
          }}
        />
      </div>
    </div>
  );
}

function RuleParamFields({
  rule,
  index,
  onChange,
}: {
  rule: ScorerRule;
  index: number;
  onChange: (next: ScorerRule) => void;
}) {
  const ui = useUiTranslations();
  switch (rule.type) {
    case "contains":
    case "not_contains":
      return (
        <div className="space-y-2">
          <Label htmlFor={`rule-text-${index}`}>{ui("Text")}</Label>
          <Input
            id={`rule-text-${index}`}
            value={rule.text}
            onChange={(e) => onChange({ ...rule, text: e.target.value })}
          />
        </div>
      );
    case "regex":
      return (
        <div className="space-y-2">
          <Label htmlFor={`rule-pattern-${index}`}>{ui("Pattern")}</Label>
          <Input
            id={`rule-pattern-${index}`}
            value={rule.pattern}
            onChange={(e) => onChange({ ...rule, pattern: e.target.value })}
          />
        </div>
      );
    case "tool_called":
      return (
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor={`rule-tool-${index}`}>{ui("Tool")}</Label>
            <Input
              id={`rule-tool-${index}`}
              value={rule.tool}
              onChange={(e) => onChange({ ...rule, tool: e.target.value })}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor={`rule-min-${index}`}>{ui("Minimum calls")}</Label>
            <Input
              id={`rule-min-${index}`}
              type="number"
              min={1}
              value={rule.min ?? 1}
              onChange={(e) => onChange({ ...rule, min: Number(e.target.value) })}
            />
          </div>
        </div>
      );
    case "tool_not_called":
      return (
        <div className="space-y-2">
          <Label htmlFor={`rule-tool-${index}`}>{ui("Tool")}</Label>
          <Input
            id={`rule-tool-${index}`}
            value={rule.tool}
            onChange={(e) => onChange({ ...rule, tool: e.target.value })}
          />
        </div>
      );
    case "tool_call_count":
      return (
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor={`rule-min-${index}`}>{ui("Min (optional)")}</Label>
            <Input
              id={`rule-min-${index}`}
              type="number"
              min={0}
              value={rule.min ?? ""}
              onChange={(e) =>
                onChange({
                  ...rule,
                  min: e.target.value === "" ? undefined : Number(e.target.value),
                })
              }
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor={`rule-max-${index}`}>{ui("Max (optional)")}</Label>
            <Input
              id={`rule-max-${index}`}
              type="number"
              min={0}
              value={rule.max ?? ""}
              onChange={(e) =>
                onChange({
                  ...rule,
                  max: e.target.value === "" ? undefined : Number(e.target.value),
                })
              }
            />
          </div>
        </div>
      );
    case "turns_within":
      return (
        <div className="space-y-2">
          <Label htmlFor={`rule-max-${index}`}>{ui("Max turns")}</Label>
          <Input
            id={`rule-max-${index}`}
            type="number"
            min={1}
            value={rule.max}
            onChange={(e) => onChange({ ...rule, max: Number(e.target.value) })}
          />
        </div>
      );
    case "json_schema":
      return (
        <div className="space-y-2">
          <Label htmlFor={`rule-schema-${index}`}>{ui("JSON schema")}</Label>
          <Textarea
            id={`rule-schema-${index}`}
            className="font-mono text-xs"
            rows={5}
            defaultValue={JSON.stringify(rule.schema, null, 2)}
            onChange={(e) => {
              try {
                const parsed = JSON.parse(e.target.value);
                onChange({ ...rule, schema: parsed });
              } catch {
                // Ignore invalid JSON while typing; keep last valid schema.
              }
            }}
          />
          <p className="text-xs text-muted-foreground">
            {ui("Must be valid JSON. Invalid input is ignored until corrected.")}
          </p>
        </div>
      );
  }
}
