// Helper models: the models an agent's spawned subagents may run on. They live
// in the `subagents` capability config (`helper_models`, `helper_model_pick`),
// mirroring crates/contracts/src/runtime/helper_models.rs.
import type { AgentCapabilityConfig } from "@/lib/api/types";
import type { ModelJob, ModelPreset } from "@/lib/api/model-presets";

// Helper models are the models a copy of the agent may run on, set on its
// Spawn agents capability.
export const SUBAGENTS_CAPABILITY = "spawn_agents";
export const SAME_KEY = "same";
export const MAX_HELPER_MODELS = 16;

export type HelperModelTarget =
  | { type: "same" }
  | { type: "job"; job: ModelJob }
  | { type: "preset"; preset: string; job: ModelJob }
  | { type: "model"; model_id: string }
  | { type: "other_provider"; job: ModelJob };

export type HelperModel = {
  key: string;
  when: string;
  target: HelperModelTarget;
};
export type HelperModelPick = "agent" | "auto";

export const BUILTIN_HELPER_MODELS: HelperModel[] = [
  {
    key: "same",
    when: "Most work. Runs on your own model.",
    target: { type: "same" },
  },
  {
    key: "fast",
    when: "Search, lookups, summaries: cheap and quick.",
    target: { type: "job", job: "small" },
  },
  {
    key: "coding",
    when: "Implementation work.",
    target: { type: "job", job: "coding" },
  },
  {
    key: "deep",
    when: "Hard reasoning and planning.",
    target: { type: "job", job: "research" },
  },
  {
    key: "second_opinion",
    when: "Verify or review work done by another agent. A different provider than yours.",
    target: { type: "other_provider", job: "default" },
  },
];

export const BUILTIN_KEYS = new Set(BUILTIN_HELPER_MODELS.map((model) => model.key));

export function helperSettings(capabilities: AgentCapabilityConfig[]): {
  enabled: boolean;
  models: HelperModel[];
  pick: HelperModelPick;
} {
  const entry = capabilities.find((c) => c.ref === SUBAGENTS_CAPABILITY);
  const config = (entry?.config ?? {}) as {
    helper_models?: HelperModel[];
    helper_model_pick?: HelperModelPick;
  };
  return {
    enabled: !!entry,
    models:
      Array.isArray(config.helper_models) && config.helper_models.length > 0
        ? config.helper_models
        : BUILTIN_HELPER_MODELS,
    pick: config.helper_model_pick === "auto" ? "auto" : "agent",
  };
}

/** The capabilities with the subagents config carrying `models` and `pick`.
 * The built-in list with the agent pick is stored as no config at all. */
export function withHelperSettings(
  capabilities: AgentCapabilityConfig[],
  models: HelperModel[],
  pick: HelperModelPick,
): AgentCapabilityConfig[] {
  const builtin = JSON.stringify(models) === JSON.stringify(BUILTIN_HELPER_MODELS);
  return capabilities.map((capability) => {
    if (capability.ref !== SUBAGENTS_CAPABILITY) return capability;
    const config = { ...capability.config };
    delete config.helper_models;
    delete config.helper_model_pick;
    if (!builtin) config.helper_models = models;
    if (pick === "auto") config.helper_model_pick = pick;
    return { ...capability, config };
  });
}

const JOB_LABELS: Record<ModelJob, string> = {
  default: "Default",
  chat: "Chat",
  coding: "Coding",
  research: "Research",
  small: "Small",
  decision: "Decision",
};

export function jobLabel(job: ModelJob): string {
  return JOB_LABELS[job] ?? job;
}

/** A short label for where a key's model comes from. */
export function targetLabel(
  target: HelperModelTarget,
  presets: ModelPreset[],
  modelName: (id: string) => string | undefined,
): string {
  switch (target.type) {
    case "same":
      return "Parent's model";
    case "job":
      return `Job: ${jobLabel(target.job)}`;
    case "preset": {
      const name = presets.find((p) => p.id === target.preset)?.name ?? target.preset;
      return `${name} preset · ${jobLabel(target.job)}`;
    }
    case "model":
      return modelName(target.model_id) ?? target.model_id;
    case "other_provider":
      return `Another provider · ${jobLabel(target.job)}`;
  }
}

/** The model a target lands on today, when the page can tell. */
export function targetModelId(
  target: HelperModelTarget,
  presets: ModelPreset[],
  agentPreset: string | undefined,
  orgJobModel: (job: ModelJob) => string | null | undefined,
): string | undefined {
  const fromPreset = (id: string | undefined, job: ModelJob) => {
    const preset = id ? presets.find((p) => p.id === id) : undefined;
    return (preset?.[`${job}_model_id` as const] as string | null | undefined) ?? undefined;
  };
  switch (target.type) {
    case "job":
      return fromPreset(agentPreset, target.job) ?? orgJobModel(target.job) ?? undefined;
    case "preset":
      return fromPreset(target.preset, target.job);
    case "model":
      return target.model_id;
    default:
      return undefined;
  }
}

/** Errors that would make the server refuse the list. */
export function helperModelErrors(models: HelperModel[]): string[] {
  const errors: string[] = [];
  const seen = new Set<string>();
  if (models.length === 0) errors.push("Keep at least one key, such as same.");
  if (models.length > MAX_HELPER_MODELS) errors.push(`At most ${MAX_HELPER_MODELS} keys.`);
  for (const model of models) {
    if (!/^[a-z0-9_-]{1,40}$/.test(model.key)) {
      errors.push(`"${model.key}" must be 1 to 40 lowercase letters, digits, _ or -.`);
    }
    if (!model.when.trim() || model.when.length > 300) {
      errors.push(`"${model.key}" needs a "when to use" line of 1 to 300 characters.`);
    }
    if (seen.has(model.key)) errors.push(`"${model.key}" is listed twice.`);
    seen.add(model.key);
    if (model.target.type === "model" && !model.target.model_id) {
      errors.push(`"${model.key}" needs a model.`);
    }
  }
  return errors;
}
