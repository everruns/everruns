// Sandbox Template types and API functions.

import { api } from "./client";
import type {
  CreateSandboxTemplateRequest,
  ReviseSandboxTemplateRequest,
  SandboxCapabilities,
  SandboxTargetDescriptor,
  SandboxTargetsResponse,
  SandboxTemplate,
  SessionSandboxResponse,
} from "./types";

export type SessionSandbox = SessionSandboxResponse;
export type {
  CreateSandboxTemplateRequest,
  SandboxCapabilities,
  SandboxTargetDescriptor,
  SandboxTemplate,
};

export async function listSandboxTargets(): Promise<SandboxTargetsResponse> {
  return (await api.get<SandboxTargetsResponse>("/v1/sandbox-targets")).data;
}

export async function listSandboxTemplates(): Promise<SandboxTemplate[]> {
  return (await api.get<SandboxTemplate[]>("/v1/sandbox-templates")).data;
}

export async function getSandboxTemplate(id: string): Promise<SandboxTemplate> {
  return (await api.get<SandboxTemplate>(`/v1/sandbox-templates/${id}`)).data;
}

export async function createSandboxTemplate(
  request: CreateSandboxTemplateRequest,
): Promise<SandboxTemplate> {
  return (await api.post<SandboxTemplate>("/v1/sandbox-templates", request)).data;
}

export async function reviseSandboxTemplate(
  id: string,
  request: ReviseSandboxTemplateRequest,
): Promise<SandboxTemplate> {
  return (await api.put<SandboxTemplate>(`/v1/sandbox-templates/${id}`, request)).data;
}

export async function archiveSandboxTemplate(id: string): Promise<void> {
  await api.delete(`/v1/sandbox-templates/${id}`);
}
