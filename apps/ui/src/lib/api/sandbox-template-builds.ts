import { api } from "./client";
import type { SandboxBuildRequest, SandboxTemplateBuild, SandboxTemplate } from "./types";

export type { SandboxBuildRequest, SandboxTemplateBuild };
export async function listSandboxTemplateBuilds(
  id: string,
  offset = 0,
): Promise<SandboxTemplateBuild[]> {
  return (
    await api.get<SandboxTemplateBuild[]>(`/v1/sandbox-templates/${id}/builds?offset=${offset}`)
  ).data;
}
export async function startSandboxTemplateBuild(
  id: string,
  body: SandboxBuildRequest,
): Promise<SandboxTemplateBuild> {
  return (await api.post<SandboxTemplateBuild>(`/v1/sandbox-templates/${id}/builds`, body)).data;
}
export async function cancelSandboxTemplateBuild(id: string): Promise<SandboxTemplateBuild> {
  return (await api.post<SandboxTemplateBuild>(`/v1/sandbox-template-builds/${id}/cancel`)).data;
}
export async function discardSandboxTemplateBuild(id: string): Promise<SandboxTemplateBuild> {
  return (await api.post<SandboxTemplateBuild>(`/v1/sandbox-template-builds/${id}/discard`)).data;
}
export async function applySandboxTemplateBuild(id: string): Promise<SandboxTemplate> {
  return (await api.post<SandboxTemplate>(`/v1/sandbox-template-builds/${id}/apply`)).data;
}
