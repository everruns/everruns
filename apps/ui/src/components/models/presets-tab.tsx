"use client";

// Models › Presets: named sets of models, one per job
// (knowledge/foundations/model-presets.md). The organization defaults stay on
// the Defaults tab; an agent uses them unless it picks a preset here.
//
// Built-in presets (Automatic and one per connected provider) are computed by
// the server from curated picks and are read only: duplicate one to change it.
// A custom preset's job left empty follows the organization default.

import { useMemo, useState } from "react";
import { Copy, Lock, Pencil, Plus, Sparkles, Trash2, Layers, Upload } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Combobox } from "@/components/ui/combobox";
import { Input } from "@/components/ui/input";
import { Notice, NoticeDescription } from "@/components/ui/notice";
import { useConfirmation } from "@/components/ui/confirmation-provider";
import { useModelPresetMutations, useModelPresets } from "@/hooks/use-model-presets";
import { ApiError } from "@/lib/api/client";
import { MODEL_JOBS, type ModelPreset } from "@/lib/api/model-presets";
import type { ModelWithProvider } from "@/lib/api/types";
import { isChatModel } from "@/lib/model-capabilities";
import { compareByRecency, modelLabel } from "@/lib/model-selection";
import { cn } from "@/lib/utils";
import { useUiTranslations } from "@/providers/locale-provider";

const ORG_DEFAULT = "__org_default__";

function errorText(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  if (error instanceof Error && error.message) return error.message;
  return "Unexpected error";
}

function kindLabel(preset: ModelPreset): string {
  if (preset.kind === "custom") return "Custom";
  if (preset.kind === "provider") return `Built in · from ${preset.name} picks`;
  return "Built in";
}

/** One line naming the models a preset uses, grouped by model. */
function summary(preset: ModelPreset, byId: Map<string, ModelWithProvider>): string {
  const groups = new Map<string, string[]>();
  for (const { field, label } of MODEL_JOBS) {
    const id = preset[field];
    if (!id) continue;
    const name = byId.get(id)?.display_name ?? id;
    groups.set(name, [...(groups.get(name) ?? []), label.toLowerCase()]);
  }
  if (groups.size === 0) return "Every job follows the organization defaults.";
  return [...groups.entries()].map(([name, jobs]) => `${name} ${jobs.join(", ")}`).join("; ");
}

export function PresetsTab({
  models,
  canManage,
  onNotice,
}: {
  models: ModelWithProvider[];
  canManage: boolean;
  onNotice: (notice: { kind: "success" | "error"; text: string }) => void;
}) {
  const ui = useUiTranslations();
  const requestConfirmation = useConfirmation();
  const { data: presets = [], isLoading, error } = useModelPresets();
  const { create, update, remove, apply } = useModelPresetMutations();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const byId = useMemo(() => new Map(models.map((model) => [model.id, model])), [models]);
  const selected = presets.find((preset) => preset.id === selectedId) ?? presets[0];

  const options = useMemo(
    () => [
      { value: ORG_DEFAULT, label: ui("Organization default") },
      ...models
        .filter(
          (model) =>
            model.enabled &&
            !model.stale &&
            isChatModel(model) &&
            model.provider_type !== "chatgpt",
        )
        .sort(compareByRecency)
        .map((model) => ({ value: model.id, label: modelLabel(model) })),
    ],
    [models, ui],
  );

  const jobModelLabel = (id: string) => {
    const model = byId.get(id);
    return model ? modelLabel(model) : id;
  };

  const busy = create.isPending || update.isPending || remove.isPending || apply.isPending;
  const run = async (action: () => Promise<unknown>, success?: string) => {
    try {
      await action();
      if (success) onNotice({ kind: "success", text: success });
    } catch (err) {
      onNotice({ kind: "error", text: errorText(err) });
    }
  };

  const duplicate = (preset: ModelPreset) =>
    run(async () => {
      const created = await create.mutateAsync({ name: `${preset.name} copy`, from: preset.id });
      setSelectedId(created.id);
      setRenaming(created.id);
    }, ui("Preset duplicated."));

  const newPreset = () =>
    run(async () => {
      const created = await create.mutateAsync({ name: "New preset" });
      setSelectedId(created.id);
      setRenaming(created.id);
    });

  const copyToDefaults = async (preset: ModelPreset) => {
    if (
      !(await requestConfirmation(
        ui("Copy the models of {value0} into the organization defaults?", { value0: preset.name }),
      ))
    )
      return;
    await run(async () => {
      const result = await apply.mutateAsync(preset.id);
      onNotice({
        kind: "success",
        text: result.skipped_jobs.length
          ? ui("Organization defaults updated. Kept as they were: {value0}.", {
              value0: result.skipped_jobs.join(", "),
            })
          : ui("Organization defaults updated."),
      });
    });
  };

  const deletePreset = async (preset: ModelPreset) => {
    if (!(await requestConfirmation(ui("Delete the preset {value0}?", { value0: preset.name }))))
      return;
    await run(async () => {
      await remove.mutateAsync(preset.id);
      setSelectedId(null);
    }, ui("Preset deleted."));
  };

  if (error) {
    return (
      <Notice variant="destructive" role="alert">
        <NoticeDescription>{errorText(error)}</NoticeDescription>
      </Notice>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <section aria-labelledby="presets-heading" className="flex flex-col gap-3">
        <div>
          <h3 id="presets-heading" className="text-sm font-semibold">
            {ui("Presets")}
          </h3>
          <p className="text-xs text-muted-foreground">
            {ui(
              "Named sets of models, one per job. An agent uses the organization defaults unless it picks a preset.",
            )}
          </p>
        </div>
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
          {isLoading && <p className="text-sm text-muted-foreground">{ui("Loading presets…")}</p>}
          {presets.map((preset) => {
            const Icon =
              preset.kind === "automatic" ? Sparkles : preset.kind === "provider" ? Layers : Copy;
            return (
              <button
                key={preset.id}
                type="button"
                aria-pressed={selected?.id === preset.id}
                onClick={() => setSelectedId(preset.id)}
                className={cn(
                  "flex flex-col gap-1.5 border bg-card p-4 text-left transition-colors hover:bg-muted/40",
                  selected?.id === preset.id && "border-primary",
                )}
              >
                <span className="flex items-center gap-2 text-sm font-semibold">
                  <Icon className="icon-sharp size-4" aria-hidden />
                  {preset.name}
                  {preset.read_only && (
                    <Lock className="size-3.5 text-muted-foreground" aria-label={ui("Read only")} />
                  )}
                </span>
                <span className="text-xs text-muted-foreground">
                  {ui(kindLabel(preset))}
                  {preset.agent_count > 0 &&
                    ` · ${ui("{value0} agents", { value0: String(preset.agent_count) })}`}
                </span>
                <span className="text-sm">{summary(preset, byId)}</span>
              </button>
            );
          })}
          {canManage && !isLoading && (
            <button
              type="button"
              onClick={() => void newPreset()}
              disabled={busy}
              className="flex min-h-24 items-center justify-center gap-2 border border-dashed text-sm text-muted-foreground hover:bg-muted/40"
            >
              <Plus className="size-4" />
              {ui("New preset")}
            </button>
          )}
        </div>
      </section>

      {selected && (
        <section aria-labelledby="preset-detail" className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 items-baseline gap-2">
              {renaming === selected.id ? (
                <Input
                  ref={(input) => input?.focus()}
                  aria-label={ui("Preset name")}
                  defaultValue={selected.name}
                  className="h-8 w-64"
                  onBlur={(event) => {
                    const name = event.currentTarget.value.trim();
                    setRenaming(null);
                    if (name && name !== selected.name)
                      void run(() => update.mutateAsync({ id: selected.id, name }));
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") event.currentTarget.blur();
                    if (event.key === "Escape") setRenaming(null);
                  }}
                />
              ) : (
                <h3 id="preset-detail" className="text-sm font-semibold">
                  {selected.name}
                </h3>
              )}
              <span className="text-xs text-muted-foreground">{ui(kindLabel(selected))}</span>
            </div>
            {canManage && (
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={busy}
                  onClick={() => void duplicate(selected)}
                >
                  <Copy className="size-4" />
                  {selected.read_only ? ui("Duplicate to customize") : ui("Duplicate")}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={busy}
                  onClick={() => void copyToDefaults(selected)}
                >
                  <Upload className="size-4" />
                  {ui("Copy to organization defaults")}
                </Button>
                {!selected.read_only && (
                  <>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={busy}
                      onClick={() => setRenaming(selected.id)}
                    >
                      <Pencil className="size-4" />
                      {ui("Rename")}
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={busy || selected.agent_count > 0}
                      title={
                        selected.agent_count > 0
                          ? ui("Agents use this preset; move them first.")
                          : undefined
                      }
                      onClick={() => void deletePreset(selected)}
                    >
                      <Trash2 className="size-4" />
                      {ui("Delete")}
                    </Button>
                  </>
                )}
              </div>
            )}
          </div>
          <div className="divide-y border bg-card">
            {MODEL_JOBS.map(({ job, field, label, hint }) => {
              const value = selected[field] ?? "";
              const fallback = selected.fallback_jobs?.includes(job);
              return (
                <div
                  key={job}
                  className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 px-4 py-3"
                >
                  <div className="min-w-0 flex-[1_1_240px]">
                    <p className="text-sm font-medium">{ui(label)}</p>
                    <p className="text-xs text-muted-foreground">{ui(hint)}</p>
                  </div>
                  <div className="w-full sm:w-80">
                    {selected.read_only || !canManage ? (
                      <p className="text-sm">
                        {value ? jobModelLabel(value) : ui("Organization default")}
                        {fallback && (
                          <span className="ml-2 text-xs text-muted-foreground">
                            {ui("next recommended model")}
                          </span>
                        )}
                      </p>
                    ) : (
                      <Combobox
                        options={options}
                        value={value || ORG_DEFAULT}
                        onValueChange={(picked) =>
                          void run(() =>
                            update.mutateAsync({
                              id: selected.id,
                              [field]: picked === ORG_DEFAULT || !picked ? null : picked,
                            }),
                          )
                        }
                        placeholder={ui("Organization default")}
                        searchPlaceholder="Search chat models…"
                        disabled={busy}
                      />
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          <p className="text-xs text-muted-foreground">
            {selected.agent_count > 0
              ? ui("Used by {value0} agents.", { value0: String(selected.agent_count) })
              : ui("No agent uses this preset yet.")}{" "}
            {selected.read_only
              ? ui("Built-in presets follow the latest recommended models.")
              : ui('A job left as "Organization default" follows the Defaults tab.')}
          </p>
        </section>
      )}
    </div>
  );
}
