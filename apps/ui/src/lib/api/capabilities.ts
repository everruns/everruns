// Capability API functions
// All routes are org-scoped: /v1/orgs/{org}/capabilities/...
//
// Note: Agent-specific capabilities are managed through the agents API.
// See agents.ts for createAgent/updateAgent with capabilities.

import { api } from "./client";
import type {
  Capability,
  CapabilityId,
  ListResponse,
} from "./types";

export async function listCapabilities(org: string): Promise<Capability[]> {
  const response = await api.get<ListResponse<Capability>>(`/v1/orgs/${org}/capabilities`);
  return response.data.data;
}

export async function getCapability(
  org: string,
  capabilityId: CapabilityId
): Promise<Capability> {
  const response = await api.get<Capability>(
    `/v1/orgs/${org}/capabilities/${capabilityId}`
  );
  return response.data;
}
