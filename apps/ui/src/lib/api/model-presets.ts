// Model presets: named sets of models, one per job (knowledge/foundations/model-presets.md).
import { api } from "./client";
import type { Organization } from "./organization-types";

export type ModelJob = "default" | "chat" | "coding" | "research" | "small" | "decision";
export type ModelPresetKind = "automatic" | "provider" | "custom";

export type ModelPresetJobs = {
  default_model_id?: string | null;
  chat_model_id?: string | null;
  coding_model_id?: string | null;
  research_model_id?: string | null;
  small_model_id?: string | null;
  decision_model_id?: string | null;
};

export type ModelPreset = ModelPresetJobs & {
  /** `automatic`, `provider:<driver>` or a custom preset id. */
  id: string;
  name: string;
  description?: string | null;
  kind: ModelPresetKind;
  provider_type?: string | null;
  read_only: boolean;
  /** Jobs of a provider preset that use the provider's next recommended model. */
  fallback_jobs?: ModelJob[];
  agent_count: number;
  created_at?: string;
  updated_at?: string;
};

export type ApplyModelPresetResult = {
  applied_jobs: ModelJob[];
  skipped_jobs: ModelJob[];
  organization: Organization;
};

export const MODEL_JOBS: {
  job: ModelJob;
  field: keyof ModelPresetJobs;
  label: string;
  hint: string;
}[] = [
  {
    job: "default",
    field: "default_model_id",
    label: "Default",
    hint: "Agents and sessions with no model of their own",
  },
  { job: "chat", field: "chat_model_id", label: "Chat", hint: "Everruns Chat" },
  {
    job: "coding",
    field: "coding_model_id",
    label: "Coding",
    hint: "Coding harness agents and coding helpers",
  },
  {
    job: "research",
    field: "research_model_id",
    label: "Research",
    hint: "Research agents and deep helpers",
  },
  {
    job: "small",
    field: "small_model_id",
    label: "Small",
    hint: "Titles, summaries, fast helpers",
  },
  {
    job: "decision",
    field: "decision_model_id",
    label: "Decision",
    hint: "Quick gate checks",
  },
];

const BASE = "/v1/model-presets";
const path = (id: string) => `${BASE}/${encodeURIComponent(id)}`;

export async function listModelPresets(): Promise<ModelPreset[]> {
  return (await api.get<{ data: ModelPreset[] }>(BASE)).data.data;
}

export async function createModelPreset(
  request: ModelPresetJobs & { name: string; description?: string | null; from?: string },
): Promise<ModelPreset> {
  return (await api.post<ModelPreset>(BASE, request)).data;
}

export async function updateModelPreset(
  id: string,
  request: ModelPresetJobs & { name?: string; description?: string | null },
): Promise<ModelPreset> {
  return (await api.patch<ModelPreset>(path(id), request)).data;
}

export async function deleteModelPreset(id: string): Promise<void> {
  await api.delete(path(id));
}

export async function applyModelPreset(id: string): Promise<ApplyModelPresetResult> {
  return (await api.post<ApplyModelPresetResult>(`${path(id)}/apply`, {})).data;
}
