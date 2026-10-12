import type { ModelService, ModelWithProvider } from "@/lib/api/types";
import { modelService } from "@/lib/model-capabilities";

// Order in which service groups are listed wherever models are grouped.
export const SERVICE_ORDER: ModelService[] = [
  "chat",
  "decisions",
  "embeddings",
  "realtime",
  "images",
  "rerank",
];

export const SERVICE_LABELS: Record<ModelService, string> = {
  chat: "Chat",
  decisions: "Decisions",
  embeddings: "Embeddings",
  realtime: "Realtime",
  images: "Images",
  rerank: "Rerank",
};

/**
 * Models a person most likely wants when a provider is connected or a sync finds
 * new ones: the newest release of each model family, plus every calibrated
 * decision model. A model with no curated profile (no family or release date)
 * is never recommended, which keeps a several-hundred-model catalog such as
 * OpenRouter down to a short, known list.
 */
export function recommendedModelIds(models: ModelWithProvider[]): Set<string> {
  const recommended = new Set<string>();
  const newestByFamily = new Map<string, ModelWithProvider>();
  for (const model of models) {
    const service = modelService(model);
    if (service === "decisions") {
      if (model.profile?.decisions?.calibrated) recommended.add(model.id);
      continue;
    }
    const family = model.profile?.family;
    const released = model.profile?.release_date;
    if (!family || !released) continue;
    const key = `${model.provider_id}\u0000${service}\u0000${family}`;
    const current = newestByFamily.get(key);
    if (!current || released > (current.profile?.release_date ?? "")) {
      newestByFamily.set(key, model);
    }
  }
  for (const model of newestByFamily.values()) recommended.add(model.id);
  return recommended;
}

/** Group models by service in {@link SERVICE_ORDER}, dropping empty groups. */
export function groupByService(
  models: ModelWithProvider[],
): { service: ModelService; models: ModelWithProvider[] }[] {
  return SERVICE_ORDER.map((service) => ({
    service,
    models: models.filter((model) => modelService(model) === service),
  })).filter((group) => group.models.length > 0);
}

/** Newest release first, then newest row; models without a release date last. */
export function compareByRecency(a: ModelWithProvider, b: ModelWithProvider): number {
  const aDate = a.profile?.release_date ?? "";
  const bDate = b.profile?.release_date ?? "";
  if (aDate !== bDate) return bDate.localeCompare(aDate);
  return (b.created_at ?? "").localeCompare(a.created_at ?? "");
}

/** `Model (Provider name)`: two providers can serve the same model id. */
export function modelLabel(model: Pick<ModelWithProvider, "display_name" | "provider_name">) {
  return `${model.display_name} (${model.provider_name})`;
}

/** The enable/disable calls that turn the current state into `selected`. */
export function selectionChanges(
  models: ModelWithProvider[],
  selected: Set<string>,
): { id: string; enabled: boolean }[] {
  return models
    .filter((model) => model.enabled !== selected.has(model.id))
    .map((model) => ({ id: model.id, enabled: selected.has(model.id) }));
}
