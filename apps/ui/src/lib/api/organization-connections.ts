import { api } from "./client";

export interface OrganizationConnection {
  id: string;
  name: string;
  provider: string;
  provider_username?: string;
  connected_at: string;
  updated_at: string;
}

export interface SaveOrganizationConnection {
  name: string;
  apiKey: string;
  extraFields?: Record<string, string>;
}

function base(org: string): string {
  return `/v1/orgs/${encodeURIComponent(org)}/organization-connections`;
}

export async function listOrganizationConnections(org: string): Promise<OrganizationConnection[]> {
  return (await api.get<OrganizationConnection[]>(base(org))).data;
}

export async function createOrganizationConnection(
  org: string,
  provider: string,
  input: SaveOrganizationConnection,
): Promise<OrganizationConnection> {
  return (
    await api.post<OrganizationConnection>(`${base(org)}/providers/${provider}`, {
      name: input.name,
      api_key: input.apiKey,
      ...input.extraFields,
    })
  ).data;
}

export async function updateOrganizationConnection(
  org: string,
  id: string,
  input: SaveOrganizationConnection,
): Promise<OrganizationConnection> {
  return (
    await api.put<OrganizationConnection>(`${base(org)}/${id}`, {
      name: input.name,
      api_key: input.apiKey,
      ...input.extraFields,
    })
  ).data;
}

export async function deleteOrganizationConnection(org: string, id: string): Promise<void> {
  await api.delete(`${base(org)}/${id}`);
}

export async function verifyOrganizationConnection(
  org: string,
  id: string,
): Promise<{ valid: boolean; error?: string }> {
  return (await api.post<{ valid: boolean; error?: string }>(`${base(org)}/${id}/verify`)).data;
}
