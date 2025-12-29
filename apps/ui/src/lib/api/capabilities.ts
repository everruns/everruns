// Capability API functions
//
// Note: Agent-specific capabilities are managed through the agents API.
// See agents.ts for createAgent/updateAgent with capabilities.

import { api } from "./client";
import type {
  Capability,
  CapabilityId,
  ListResponse,
} from "./types";

export async function listCapabilities(): Promise<Capability[]> {
  const response = await api.get<ListResponse<Capability>>("/v1/capabilities");
  return response.data.data;
}

export async function getCapability(
  capabilityId: CapabilityId
): Promise<Capability> {
  const response = await api.get<Capability>(
    `/v1/capabilities/${capabilityId}`
  );
  return response.data;
}
