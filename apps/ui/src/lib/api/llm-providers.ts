// LLM Provider and Model API functions
// All routes are org-scoped: /v1/orgs/{org}/llm-providers/... and /v1/orgs/{org}/llm-models/...

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
  SyncModelsResponse,
} from "./types";

// Provider CRUD
export async function getLlmProviders(org: string): Promise<LlmProvider[]> {
  const response = await api.get<ListResponse<LlmProvider>>(`/v1/orgs/${org}/llm-providers`);
  return response.data.data;
}

export async function getLlmProvider(org: string, providerId: string): Promise<LlmProvider> {
  const response = await api.get<LlmProvider>(`/v1/orgs/${org}/llm-providers/${providerId}`);
  return response.data;
}

export async function createLlmProvider(
  org: string,
  data: CreateLlmProviderRequest
): Promise<LlmProvider> {
  const response = await api.post<LlmProvider>(`/v1/orgs/${org}/llm-providers`, data);
  return response.data;
}

export async function updateLlmProvider(
  org: string,
  providerId: string,
  data: UpdateLlmProviderRequest
): Promise<LlmProvider> {
  const response = await api.patch<LlmProvider>(`/v1/orgs/${org}/llm-providers/${providerId}`, data);
  return response.data;
}

export async function deleteLlmProvider(org: string, providerId: string): Promise<void> {
  await api.delete(`/v1/orgs/${org}/llm-providers/${providerId}`);
}

export async function syncProviderModels(
  org: string,
  providerId: string
): Promise<SyncModelsResponse> {
  const response = await api.post<SyncModelsResponse>(
    `/v1/orgs/${org}/llm-providers/${providerId}/sync-models`
  );
  return response.data;
}

// Model CRUD
export async function getLlmModels(org: string): Promise<LlmModelWithProvider[]> {
  const response = await api.get<ListResponse<LlmModelWithProvider>>(`/v1/orgs/${org}/llm-models`);
  return response.data.data;
}

export async function getLlmProviderModels(
  org: string,
  providerId: string
): Promise<LlmModel[]> {
  const response = await api.get<ListResponse<LlmModel>>(`/v1/orgs/${org}/llm-providers/${providerId}/models`);
  return response.data.data;
}

export async function getLlmModel(org: string, modelId: string): Promise<LlmModelWithProvider> {
  const response = await api.get<LlmModelWithProvider>(`/v1/orgs/${org}/llm-models/${modelId}`);
  return response.data;
}

export async function createLlmModel(
  org: string,
  providerId: string,
  data: CreateLlmModelRequest
): Promise<LlmModel> {
  const response = await api.post<LlmModel>(`/v1/orgs/${org}/llm-providers/${providerId}/models`, data);
  return response.data;
}

export async function updateLlmModel(
  org: string,
  modelId: string,
  data: UpdateLlmModelRequest
): Promise<LlmModel> {
  const response = await api.patch<LlmModel>(`/v1/orgs/${org}/llm-models/${modelId}`, data);
  return response.data;
}

export async function deleteLlmModel(org: string, modelId: string): Promise<void> {
  await api.delete(`/v1/orgs/${org}/llm-models/${modelId}`);
}
