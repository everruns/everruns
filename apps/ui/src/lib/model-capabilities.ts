import type { Model, ModelService } from "@/lib/api/types";
type ModelCapabilities = Pick<Model, "capabilities" | "service">;
const SERVICES: ModelService[] = [
  "chat",
  "decisions",
  "embeddings",
  "realtime",
  "images",
  "rerank",
];
export function modelService(model: ModelCapabilities): ModelService {
  if (model.service) return model.service;
  const tags = model.capabilities.map((tag) => tag.toLowerCase());
  return SERVICES.find((service) => service !== "chat" && tags.includes(service)) ?? "chat";
}
export function matchesModelService(model: ModelCapabilities, service: ModelService): boolean {
  return modelService(model) === service;
}
export function isEmbeddingModel(model: ModelCapabilities): boolean {
  return matchesModelService(model, "embeddings");
}
export function isDecisionModel(model: ModelCapabilities): boolean {
  return matchesModelService(model, "decisions");
}
export function isChatModel(model: ModelCapabilities): boolean {
  return matchesModelService(model, "chat");
}
