import { apiClient } from "./client";
import type {
  LlmProvider,
  LlmModel,
  LlmModelWithProvider,
  CreateLlmProviderRequest,
  UpdateLlmProviderRequest,
  CreateLlmModelRequest,
  UpdateLlmModelRequest,
} from "./types";

// Provider CRUD
export async function getLlmProviders(): Promise<LlmProvider[]> {
  return apiClient<LlmProvider[]>("/v1/llm-providers");
}

export async function getLlmProvider(providerId: string): Promise<LlmProvider> {
  return apiClient<LlmProvider>(`/v1/llm-providers/${providerId}`);
}

export async function createLlmProvider(
  data: CreateLlmProviderRequest
): Promise<LlmProvider> {
  return apiClient<LlmProvider>("/v1/llm-providers", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function updateLlmProvider(
  providerId: string,
  data: UpdateLlmProviderRequest
): Promise<LlmProvider> {
  return apiClient<LlmProvider>(`/v1/llm-providers/${providerId}`, {
    method: "PATCH",
    body: JSON.stringify(data),
  });
}

export async function deleteLlmProvider(providerId: string): Promise<void> {
  await apiClient(`/v1/llm-providers/${providerId}`, {
    method: "DELETE",
  });
}

// Model CRUD
export async function getLlmModels(): Promise<LlmModelWithProvider[]> {
  return apiClient<LlmModelWithProvider[]>("/v1/llm-models");
}

export async function getLlmProviderModels(
  providerId: string
): Promise<LlmModel[]> {
  return apiClient<LlmModel[]>(`/v1/llm-providers/${providerId}/models`);
}

export async function getLlmModel(modelId: string): Promise<LlmModel> {
  return apiClient<LlmModel>(`/v1/llm-models/${modelId}`);
}

export async function createLlmModel(
  providerId: string,
  data: CreateLlmModelRequest
): Promise<LlmModel> {
  return apiClient<LlmModel>(`/v1/llm-providers/${providerId}/models`, {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function updateLlmModel(
  modelId: string,
  data: UpdateLlmModelRequest
): Promise<LlmModel> {
  return apiClient<LlmModel>(`/v1/llm-models/${modelId}`, {
    method: "PATCH",
    body: JSON.stringify(data),
  });
}

export async function deleteLlmModel(modelId: string): Promise<void> {
  await apiClient(`/v1/llm-models/${modelId}`, {
    method: "DELETE",
  });
}
