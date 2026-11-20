"use client";

// The helper model a subagent ran on: its key, the model, and who picked it
// (task spec `model`, `model_id`, `model_pick`, `model_pick_reason`; see
// knowledge/foundations/model-presets.md, "Helper models").

import { useModels } from "@/hooks";
import { useUiTranslations } from "@/providers/locale-provider";

export type HelperModelSpec = {
  model?: string;
  model_id?: string | null;
  model_pick?: "agent" | "auto" | "default";
  model_pick_reason?: string | null;
};

/** The spec's helper model fields, when the task recorded a helper key. */
export function helperModelSpec(spec: unknown): HelperModelSpec | undefined {
  if (!spec || typeof spec !== "object") return undefined;
  const value = spec as HelperModelSpec;
  return typeof value.model === "string" ? value : undefined;
}

/** "Agent picked", "Auto: <reason>", or nothing for the default `same`. */
export function helperModelPickLabel(spec: HelperModelSpec): string | undefined {
  if (spec.model_pick === "agent") return "agent picked";
  if (spec.model_pick === "auto") {
    return spec.model_pick_reason ? `Auto: ${spec.model_pick_reason}` : "Auto";
  }
  return undefined;
}

export function HelperModelChip({ spec }: { spec: HelperModelSpec }) {
  const ui = useUiTranslations();
  const { data: models = [] } = useModels();
  const name = spec.model_id
    ? (models.find((model) => model.id === spec.model_id)?.display_name ?? undefined)
    : undefined;
  const pick = helperModelPickLabel(spec);
  return (
    <span className="inline-flex items-center gap-2 text-xs text-muted-foreground">
      <span
        className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5"
        data-testid="helper-model-chip"
      >
        <span className="font-mono text-foreground">{spec.model}</span>
        {name && <span>· {name}</span>}
      </span>
      {pick && <span>{pick === "agent picked" ? ui("agent picked") : pick}</span>}
    </span>
  );
}
