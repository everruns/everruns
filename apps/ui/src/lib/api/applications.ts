import { api } from "./client";
export type ApplicationResource = {
  slug: string;
  name: string;
  kind: "chat" | "room" | "memory";
  resource_id: string;
  writable: boolean;
};
export type ApplicationInput = {
  name: string;
  slug: string;
  description: string;
  config: {
    membership: "open" | "invites" | "org_users";
    invites: string[];
    show_in_sidebar: boolean;
    automations: boolean;
  };
  resources: ApplicationResource[];
};
export type Application = ApplicationInput & {
  id: string;
  status: "draft" | "published" | "archived";
  version: number;
  public_url: string | null;
  created_at: string;
  updated_at: string;
};
export const listApplications = async (includeArchived = false) =>
  (await api.get<Application[]>(`/v1/platform/applications?include_archived=${includeArchived}`))
    .data;
export const getApplication = async (id: string) =>
  (await api.get<Application>(`/v1/platform/applications/${id}`)).data;
export const createApplication = async (input: ApplicationInput) =>
  (await api.post<Application>("/v1/platform/applications", input)).data;
export const updateApplication = async (id: string, input: ApplicationInput, version: number) =>
  (await api.patch<Application>(`/v1/platform/applications/${id}`, { ...input, version })).data;
export const setApplicationStatus = async (
  id: string,
  status: Application["status"],
  version: number,
) =>
  (await api.post<Application>(`/v1/platform/applications/${id}/status`, { status, version })).data;
export const listApplicationLaunchers = async () =>
  (await api.get<{ id: string; name: string; url: string }[]>("/v1/platform/application-launchers"))
    .data;

export const createApplicationResource = async (
  id: string,
  input: {
    version: number;
    kind: ApplicationResource["kind"];
    name: string;
    slug: string;
    agent_id?: string;
  },
) => (await api.post<Application>(`/v1/platform/applications/${id}/resources`, input)).data;
