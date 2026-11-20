"use client";

// The agent's helper models: which models its spawned subagents may run on
// (knowledge/foundations/model-presets.md, "Helper models"). Shown on the
// agent page when the agent has the Subagents capability. The list and who
// picks a key live in the `subagents` capability config, so editing them is a
// draft change like any capability edit: nothing saves until Save.

import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useModelPresets } from "@/hooks/use-model-presets";
import { useModels } from "@/hooks";
import { useOrganization } from "@/hooks/use-organizations";
import { useUiTranslations } from "@/providers/locale-provider";
import type { AgentCapabilityConfig } from "@/lib/api/types";
import type { ModelJob } from "@/lib/api/model-presets";
import {
  BUILTIN_KEYS,
  helperModelErrors,
  helperSettings,
  jobLabel,
  targetLabel,
  targetModelId,
  withHelperSettings,
  type HelperModel,
  type HelperModelPick,
  type HelperModelTarget,
} from "@/lib/helper-models";
import { cn } from "@/lib/utils";

const JOBS: ModelJob[] = [
  "default",
  "coding",
  "research",
  "small",
  "chat",
  "decision",
];

/** One select value per target, so targets fit a single Select. */
function encodeTarget(target: HelperModelTarget): string {
  switch (target.type) {
    case "same":
      return "same";
    case "job":
      return `job:${target.job}`;
    case "other_provider":
      return `other:${target.job}`;
    case "preset":
      return `preset:${target.job}:${target.preset}`;
    case "model":
      return `model:${target.model_id}`;
  }
}

function decodeTarget(value: string): HelperModelTarget {
  const [kind, first, ...rest] = value.split(":");
  switch (kind) {
    case "job":
      return { type: "job", job: first as ModelJob };
    case "other":
      return { type: "other_provider", job: first as ModelJob };
    case "preset":
      return { type: "preset", job: first as ModelJob, preset: rest.join(":") };
    case "model":
      return { type: "model", model_id: [first, ...rest].join(":") };
    default:
      return { type: "same" };
  }
}

interface HelperModelsSectionProps {
  capabilities: AgentCapabilityConfig[];
  agentPreset?: string;
  readOnly: boolean;
  onChange: (capabilities: AgentCapabilityConfig[]) => void;
}

export function HelperModelsSection({
  capabilities,
  agentPreset,
  readOnly,
  onChange,
}: HelperModelsSectionProps) {
  const ui = useUiTranslations();
  const { data: presets = [] } = useModelPresets();
  const { data: models = [] } = useModels();
  const { data: organization } = useOrganization();
  const [open, setOpen] = useState(false);
  const settings = helperSettings(capabilities);
  if (!settings.enabled) return null;

  const modelName = (id: string) =>
    models.find((model) => model.id === id)?.display_name;
  const orgJobModel = (job: ModelJob) => {
    if (!organization) return undefined;
    const org = organization as unknown as Record<
      string,
      string | null | undefined
    >;
    if (job === "research")
      return org.effective_research_model_id ?? org.research_model_id;
    if (job === "coding") return org.coding_model_id ?? org.default_model_id;
    return org[`${job}_model_id`];
  };
  const describe = (target: HelperModelTarget) => {
    const label = ui(targetLabel(target, presets, modelName));
    if (target.type === "model") return label;
    const id = targetModelId(target, presets, agentPreset, orgJobModel);
    const name = id ? modelName(id) : undefined;
    return name ? `${label} · ${name}` : label;
  };

  return (
    <div className="flex flex-col gap-1.5" data-testid="helper-models">
      <span className="text-xs font-medium text-muted-foreground">
        {ui("Helper models")} ·{" "}
        {settings.pick === "auto" ? ui("Auto picks") : ui("agent picks")}
      </span>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        {settings.models.map((model) => (
          <div key={model.key} className="contents">
            <dt className="font-mono text-foreground">{model.key}</dt>
            <dd className="truncate text-muted-foreground" title={model.when}>
              {describe(model.target)}
            </dd>
          </div>
        ))}
      </dl>
      <span className="text-xs text-muted-foreground">
        {ui("What its subagents may run on.")}{" "}
        {!readOnly && (
          <button
            type="button"
            className="underline underline-offset-2 hover:text-foreground"
            onClick={() => setOpen(true)}
          >
            {ui("Edit")}
          </button>
        )}
      </span>
      {open && (
        <HelperModelsSheet
          initialModels={settings.models}
          initialPick={settings.pick}
          onClose={() => setOpen(false)}
          onSave={(next, pick) => {
            onChange(withHelperSettings(capabilities, next, pick));
            setOpen(false);
          }}
          presets={presets.map((preset) => ({
            id: preset.id,
            name: preset.name,
          }))}
          models={models.map((model) => ({
            id: model.id,
            name: model.display_name,
          }))}
        />
      )}
    </div>
  );
}

function HelperModelsSheet({
  initialModels,
  initialPick,
  presets,
  models,
  onClose,
  onSave,
}: {
  initialModels: HelperModel[];
  initialPick: HelperModelPick;
  presets: { id: string; name: string }[];
  models: { id: string; name: string }[];
  onClose: () => void;
  onSave: (models: HelperModel[], pick: HelperModelPick) => void;
}) {
  const ui = useUiTranslations();
  const [rows, setRows] = useState<HelperModel[]>(initialModels);
  const [pick, setPick] = useState<HelperModelPick>(initialPick);
  const errors = helperModelErrors(rows);
  const update = (index: number, patch: Partial<HelperModel>) =>
    setRows((current) =>
      current.map((row, i) => (i === index ? { ...row, ...patch } : row)),
    );

  const targetOptions: { value: string; label: string }[] = [
    { value: "same", label: ui("Parent's model") },
    ...JOBS.map((job) => ({
      value: `job:${job}`,
      label: `${ui("Job")}: ${ui(jobLabel(job))}`,
    })),
    ...JOBS.map((job) => ({
      value: `other:${job}`,
      label: `${ui("Another provider")} · ${ui(jobLabel(job))}`,
    })),
    ...presets.flatMap((preset) =>
      JOBS.map((job) => ({
        value: `preset:${job}:${preset.id}`,
        label: `${preset.name} ${ui("preset")} · ${ui(jobLabel(job))}`,
      })),
    ),
    ...models.map((model) => ({
      value: `model:${model.id}`,
      label: model.name,
    })),
  ];

  return (
    <Drawer open onOpenChange={(next) => !next && onClose()}>
      <DrawerContent
        scopeToPage
        className="gap-0 overflow-y-auto p-0 sm:max-w-xl"
      >
        <DrawerHeader className="border-b p-5 pr-12">
          <DrawerTitle>{ui("Helper models")}</DrawerTitle>
          <DrawerDescription>
            {ui(
              'Models this agent\'s subagents may run on. The agent sees each key and its "when to use" line, never raw model names.',
            )}
          </DrawerDescription>
        </DrawerHeader>
        <div className="flex flex-col gap-5 p-5">
          <fieldset className="flex flex-col gap-2">
            <legend className="mb-2 text-xs font-medium text-muted-foreground">
              {ui("Who picks")}
            </legend>
            {(
              [
                [
                  "agent",
                  "The agent picks",
                  "It reads the list below and names a key when it spawns. No key means same.",
                ],
                [
                  "auto",
                  "Auto",
                  "When the agent names no key, the organization's Decision model reads the task and picks one. One small extra model call per subagent; the pick and its reason show on the subagent.",
                ],
              ] as const
            ).map(([value, title, hint]) => (
              <label
                key={value}
                className={cn(
                  "flex cursor-pointer gap-3 rounded-md border p-3",
                  pick === value && "border-foreground",
                )}
              >
                <input
                  type="radio"
                  name="helper-model-pick"
                  className="mt-1"
                  checked={pick === value}
                  onChange={() => setPick(value)}
                />
                <span className="flex flex-col gap-0.5">
                  <span className="text-sm font-medium">{ui(title)}</span>
                  <span className="text-xs text-muted-foreground">
                    {ui(hint)}
                  </span>
                </span>
              </label>
            ))}
          </fieldset>

          <div className="flex flex-col gap-2">
            <Label className="text-xs font-medium text-muted-foreground">
              {ui("Keys")}
            </Label>
            {rows.map((row, index) => {
              const builtIn =
                BUILTIN_KEYS.has(row.key) &&
                initialModels.some((m) => m.key === row.key);
              return (
                <div
                  key={index}
                  className="flex flex-col gap-2 rounded-md border p-3"
                >
                  <div className="flex items-center gap-2">
                    <Input
                      aria-label={ui("Key")}
                      value={row.key}
                      disabled={builtIn}
                      onChange={(event) =>
                        update(index, { key: event.target.value.trim() })
                      }
                      className="w-40 font-mono"
                    />
                    <Select
                      value={encodeTarget(row.target)}
                      onValueChange={(value) =>
                        value && update(index, { target: decodeTarget(value) })
                      }
                    >
                      <SelectTrigger
                        aria-label={ui("Target")}
                        className="flex-1 bg-background"
                      >
                        <SelectValue>
                          {targetOptions.find(
                            (o) => o.value === encodeTarget(row.target),
                          )?.label ?? encodeTarget(row.target)}
                        </SelectValue>
                      </SelectTrigger>
                      <SelectContent>
                        {targetOptions.map((option) => (
                          <SelectItem key={option.value} value={option.value}>
                            {option.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={ui("Remove {key}", { key: row.key })}
                      onClick={() =>
                        setRows((current) =>
                          current.filter((_, i) => i !== index),
                        )
                      }
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  </div>
                  <Input
                    aria-label={ui("When to use")}
                    placeholder={ui("When to use")}
                    value={row.when}
                    onChange={(event) =>
                      update(index, { when: event.target.value })
                    }
                  />
                </div>
              );
            })}
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              onClick={() =>
                setRows((current) => [
                  ...current,
                  {
                    key: `helper_${current.length + 1}`,
                    when: "",
                    target: { type: "job", job: "default" },
                  },
                ])
              }
            >
              <Plus className="size-4" />
              {ui("Add helper model")}
            </Button>
            <p className="text-xs text-muted-foreground">
              {ui(
                "A target is a job (from the agent's preset or the organization defaults), a job in another preset, or a model from this organization. Keep only same to stop subagents from switching models.",
              )}
            </p>
            {errors.length > 0 && (
              <ul className="text-xs text-destructive">
                {errors.map((error) => (
                  <li key={error}>{error}</li>
                ))}
              </ul>
            )}
          </div>
        </div>
        <DrawerFooter className="flex-row gap-2 border-t p-4">
          <Button
            disabled={errors.length > 0}
            onClick={() => onSave(rows, pick)}
          >
            {ui("Apply")}
          </Button>
          <Button variant="ghost" onClick={onClose}>
            {ui("Cancel")}
          </Button>
        </DrawerFooter>
      </DrawerContent>
    </Drawer>
  );
}
