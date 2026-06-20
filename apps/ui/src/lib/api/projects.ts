// Project API functions
// Routes: GET/POST /v1/projects, GET/PATCH/DELETE /v1/projects/{project}

import { api } from "./client";
import type { CreateProjectRequest, ListResponse, Project, UpdateProjectRequest } from "./types";

/** List projects in the active organization. */
export async function listProjects(): Promise<Project[]> {
  const response = await api.get<ListResponse<Project>>("/v1/projects");
  return response.data.data;
}

export async function getProject(project: string): Promise<Project> {
  const response = await api.get<Project>(`/v1/projects/${project}`);
  return response.data;
}

export async function createProject(data: CreateProjectRequest): Promise<Project> {
  const response = await api.post<Project>("/v1/projects", data);
  return response.data;
}

export async function updateProject(project: string, data: UpdateProjectRequest): Promise<Project> {
  const response = await api.patch<Project>(`/v1/projects/${project}`, data);
  return response.data;
}

export async function deleteProject(project: string): Promise<void> {
  await api.delete(`/v1/projects/${project}`);
}
