// Skills registry API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, throwApiError } from "./client";
import { createCrudApi } from "./crud";
import type {
  CreateSkillRequest,
  Skill,
  SkillContent,
  SkillValidationResult,
  UpdateSkillRequest,
  ValidateSkillRequest,
} from "./types";

export const skillsCrudApi = createCrudApi<Skill, CreateSkillRequest, UpdateSkillRequest>(
  "/v1/skills",
);

export const getSkills = skillsCrudApi.list;
export const getSkill = skillsCrudApi.get;
export const createSkill = skillsCrudApi.create;
export const updateSkill = skillsCrudApi.update;
export const deleteSkill = skillsCrudApi.delete;
export const destroySkill = skillsCrudApi.destroy;

export async function getSkillContent(skillId: string): Promise<SkillContent> {
  const response = await api.get<SkillContent>(`/v1/skills/${skillId}/content`);
  return response.data;
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
