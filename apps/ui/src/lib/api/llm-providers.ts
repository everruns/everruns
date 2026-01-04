import { api } from "./client";
import type {
  LlmProvider,
  LlmModel,
  LlmModelWithProvider,
  CreateLlmProviderRequest,
  UpdateLlmProviderRequest,
  CreateLlmModelRequest,
  UpdateLlmModelRequest,
  ListResponse,
} from "./types";

// Provider CRUD
export async function getLlmProviders(): Promise<LlmProvider[]> {
  const response = await api.get<ListResponse<LlmProvider>>("/v1/llm-providers");
  return response.data.data;
}

export async function getLlmProvider(providerId: string): Promise<LlmProvider> {
  const response = await api.get<LlmProvider>(`/v1/llm-providers/${providerId}`);
  return response.data;
}

export async function createLlmProvider(
  data: CreateLlmProviderRequest
): Promise<LlmProvider> {
  const response = await api.post<LlmProvider>("/v1/llm-providers", data);
  return response.data;
}

export async function updateLlmProvider(
  providerId: string,
  data: UpdateLlmProviderRequest
): Promise<LlmProvider> {
  const response = await api.patch<LlmProvider>(`/v1/llm-providers/${providerId}`, data);
  return response.data;
}

export async function deleteLlmProvider(providerId: string): Promise<void> {
  await api.delete(`/v1/llm-providers/${providerId}`);
}

// Model CRUD
export async function getLlmModels(): Promise<LlmModelWithProvider[]> {
  const response = await api.get<LlmModelWithProvider[]>("/v1/llm-models");
  return response.data;
}

export async function getLlmProviderModels(
  providerId: string
): Promise<LlmModel[]> {
  const response = await api.get<LlmModel[]>(`/v1/llm-providers/${providerId}/models`);
  return response.data;
}

export async function getLlmModel(modelId: string): Promise<LlmModelWithProvider> {
  const response = await api.get<LlmModelWithProvider>(`/v1/llm-models/${modelId}`);
  return response.data;
}

export async function createLlmModel(
  providerId: string,
  data: CreateLlmModelRequest
): Promise<LlmModel> {
  const response = await api.post<LlmModel>(`/v1/llm-providers/${providerId}/models`, data);
  return response.data;
}

export async function updateLlmModel(
  modelId: string,
  data: UpdateLlmModelRequest
): Promise<LlmModel> {
  const response = await api.patch<LlmModel>(`/v1/llm-models/${modelId}`, data);
  return response.data;
}

export async function deleteLlmModel(modelId: string): Promise<void> {
  await api.delete(`/v1/llm-models/${modelId}`);
}
