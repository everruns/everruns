import type { Model } from "@/lib/api/types";

type ModelCapabilities = Pick<Model, "capabilities">;

export function isEmbeddingModel(model: ModelCapabilities): boolean {
  return model.capabilities.some((capability) => capability.toLowerCase() === "embeddings");
}

export function isChatModel(model: ModelCapabilities): boolean {
  return !isEmbeddingModel(model);
}
