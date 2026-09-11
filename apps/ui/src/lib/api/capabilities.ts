// Capability API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)
//
// Note: Agent-specific capabilities are managed through the agents API.
// See agents.ts for createAgent/updateAgent with capabilities.

import { api } from "./client";
import type {
  Capability,
  CapabilityId,
  CreateDeclarativeCapabilityRequest,
  DeclarativeCapability,
  ListResponse,
  UpdateDeclarativeCapabilityRequest,
} from "./types";

/**
 * Retired capabilities are excluded by default, matching every other catalog
 * surface. Pass `includeRetired` on screens that render the capabilities an
 * existing agent or harness already references, so a retired one can be named
 * and removed instead of showing up as an unknown reference.
 */
export async function listCapabilities(includeRetired = false): Promise<Capability[]> {
  const response = await api.get<ListResponse<Capability>>(
    includeRetired ? "/v1/capabilities?include_retired=true" : "/v1/capabilities",
  );
  return response.data.data;
}

export async function getCapability(capabilityId: CapabilityId): Promise<Capability> {
  const response = await api.get<Capability>(`/v1/capabilities/${capabilityId}`);
  return response.data;
}

export async function listDeclarativeCapabilities(): Promise<DeclarativeCapability[]> {
  const response = await api.get<ListResponse<DeclarativeCapability>>(
    "/v1/capabilities/declarative",
  );
  return response.data.data;
}

export async function getDeclarativeCapability(id: string): Promise<DeclarativeCapability> {
  const response = await api.get<DeclarativeCapability>(`/v1/capabilities/declarative/${id}`);
  return response.data;
}

export async function createDeclarativeCapability(
  request: CreateDeclarativeCapabilityRequest,
): Promise<DeclarativeCapability> {
  const response = await api.post<DeclarativeCapability>("/v1/capabilities", request);
  return response.data;
}

export async function updateDeclarativeCapability(
  id: string,
  request: UpdateDeclarativeCapabilityRequest,
): Promise<DeclarativeCapability> {
  const response = await api.patch<DeclarativeCapability>(
    `/v1/capabilities/declarative/${id}`,
    request,
  );
  return response.data;
}

export async function deleteDeclarativeCapability(id: string): Promise<void> {
  await api.delete(`/v1/capabilities/declarative/${id}`);
}
