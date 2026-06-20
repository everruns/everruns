// LLM Provider and Model API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, getBackendUrl } from "./client";
import type {
  Provider,
  Model,
  ModelWithProvider,
  CreateProviderRequest,
  UpdateProviderRequest,
  CreateModelRequest,
  UpdateModelRequest,
  ListResponse,
  SyncModelsResponse,
} from "./types";

// Provider CRUD
export async function getProviders(): Promise<Provider[]> {
  const response = await api.get<ListResponse<Provider>>("/v1/providers");
  return response.data.data;
}

export async function getProvider(providerId: string): Promise<Provider> {
  const response = await api.get<Provider>(`/v1/providers/${providerId}`);
  return response.data;
}

export async function createProvider(data: CreateProviderRequest): Promise<Provider> {
  const response = await api.post<Provider>("/v1/providers", data);
  return response.data;
}

export async function updateProvider(
  providerId: string,
  data: UpdateProviderRequest,
): Promise<Provider> {
  const response = await api.patch<Provider>(`/v1/providers/${providerId}`, data);
  return response.data;
}

export async function deleteProvider(providerId: string): Promise<void> {
  await api.delete(`/v1/providers/${providerId}`);
}

export async function syncProviderModels(providerId: string): Promise<SyncModelsResponse> {
  const response = await api.post<SyncModelsResponse>(`/v1/providers/${providerId}/sync-models`);
  return response.data;
}

// Driver ids whose backend driver declares an interactive OAuth connect flow.
// This mirrors `DriverDescriptor::oauth` on the server, which stays the source
// of truth: `authorize` rejects drivers that do not declare it. It is kept
// client-side deliberately — the whole driver list, key placeholders, and base
// URLs are already hardcoded here, and the Add-Provider hint needs per-driver
// capability before any provider row exists. Folding all of this onto a backend
// driver-capability catalog endpoint is a separate, larger change (tracked as a
// follow-up); until then this single-entry set is the smallest consistent shim.
const OAUTH_PROVIDER_TYPES = new Set<string>(["openrouter"]);

export function providerSupportsOAuth(providerType: string): boolean {
  return OAUTH_PROVIDER_TYPES.has(providerType);
}

// Browser-navigation URL that begins the provider's OAuth connect flow. This is
// a server redirect (sets a state cookie), so it must hit the backend directly
// rather than going through the API client.
export function providerOAuthAuthorizeUrl(providerId: string): string {
  return `${getBackendUrl()}/v1/providers/${providerId}/oauth/authorize`;
}

// Model CRUD
export async function getModels(): Promise<ModelWithProvider[]> {
  const response = await api.get<ListResponse<ModelWithProvider>>("/v1/models");
  return response.data.data;
}

export async function getProviderModels(providerId: string): Promise<Model[]> {
  const response = await api.get<ListResponse<Model>>(`/v1/providers/${providerId}/models`);
  return response.data.data;
}

export async function getModel(modelId: string): Promise<ModelWithProvider> {
  const response = await api.get<ModelWithProvider>(`/v1/models/${modelId}`);
  return response.data;
}

export async function createModel(providerId: string, data: CreateModelRequest): Promise<Model> {
  const response = await api.post<Model>(`/v1/providers/${providerId}/models`, data);
  return response.data;
}

export async function updateModel(modelId: string, data: UpdateModelRequest): Promise<Model> {
  const response = await api.patch<Model>(`/v1/models/${modelId}`, data);
  return response.data;
}

export async function deleteModel(modelId: string): Promise<void> {
  await api.delete(`/v1/models/${modelId}`);
}
