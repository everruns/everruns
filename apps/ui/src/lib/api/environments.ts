// Environment types, from the generated OpenAPI schema.
//
// The aliases keep call sites readable without giving the UI a second opinion
// about the wire shape.

import { api } from "./client";
import type {
  CreateEnvironmentRequest,
  Environment,
  EnvironmentCapabilities,
  EnvironmentTargetsResponse,
  ReviseEnvironmentRequest,
  SessionEnvironmentResponse,
} from "./types";

export type EnvironmentDefinition = Environment;
export type CreateEnvironmentDefinitionRequest = CreateEnvironmentRequest;

export type SessionEnvironment = SessionEnvironmentResponse;
export type { EnvironmentCapabilities };
export type { EnvironmentTargetDescriptor } from "./types";

export async function listEnvironmentTargets(): Promise<EnvironmentTargetsResponse> {
  const response = await api.get<EnvironmentTargetsResponse>("/v1/environment-targets");
  return response.data;
}

export async function listEnvironments(): Promise<EnvironmentDefinition[]> {
  return (await api.get<EnvironmentDefinition[]>("/v1/environments")).data;
}

export async function getEnvironment(id: string): Promise<EnvironmentDefinition> {
  return (await api.get<EnvironmentDefinition>(`/v1/environments/${id}`)).data;
}

export async function createEnvironment(
  request: CreateEnvironmentDefinitionRequest,
): Promise<EnvironmentDefinition> {
  return (await api.post<EnvironmentDefinition>("/v1/environments", request)).data;
}

export async function reviseEnvironment(
  id: string,
  request: ReviseEnvironmentRequest,
): Promise<EnvironmentDefinition> {
  return (await api.put<EnvironmentDefinition>(`/v1/environments/${id}`, request)).data;
}

export async function archiveEnvironment(id: string): Promise<void> {
  await api.delete(`/v1/environments/${id}`);
}
