// Skills registry API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, throwApiError } from "./client";
import type {
  Skill,
  SkillContent,
  SkillValidationResult,
  CreateSkillRequest,
  UpdateSkillRequest,
  ValidateSkillRequest,
  ListResponse,
} from "./types";

// Skill CRUD

export async function getSkills(includeArchived = false): Promise<Skill[]> {
  const path = includeArchived ? "/v1/skills?include_archived=true" : "/v1/skills";
  const response = await api.get<ListResponse<Skill>>(path);
  return response.data.data;
}

export async function getSkill(skillId: string): Promise<Skill> {
  const response = await api.get<Skill>(`/v1/skills/${skillId}`);
  return response.data;
}

export async function getSkillContent(skillId: string): Promise<SkillContent> {
  const response = await api.get<SkillContent>(`/v1/skills/${skillId}/content`);
  return response.data;
}

export async function createSkill(data: CreateSkillRequest): Promise<Skill> {
  const response = await api.post<Skill>("/v1/skills", data);
  return response.data;
}

export async function updateSkill(skillId: string, data: UpdateSkillRequest): Promise<Skill> {
  const response = await api.patch<Skill>(`/v1/skills/${skillId}`, data);
  return response.data;
}

export async function deleteSkill(skillId: string): Promise<void> {
  await api.delete(`/v1/skills/${skillId}`);
}

export async function destroySkill(skillId: string): Promise<void> {
  await api.post(`/v1/skills/${skillId}/delete`);
}

export async function validateSkill(data: ValidateSkillRequest): Promise<SkillValidationResult> {
  const response = await api.post<SkillValidationResult>("/v1/skills/validate", data);
  return response.data;
}

export async function uploadSkillArchive(file: File): Promise<Skill> {
  const formData = new FormData();
  formData.append("file", file);

  // Raw fetch needed for FormData (no Content-Type header — browser sets multipart boundary)
  const response = await fetch("/api/v1/skills/upload", {
    method: "POST",
    credentials: "include",
    body: formData,
  });

  if (!response.ok) {
    await throwApiError(response);
  }

  return response.json();
}
