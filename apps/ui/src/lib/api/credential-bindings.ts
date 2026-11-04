import { api } from "./client";

export type CredentialScope = "session" | "agent" | "virtual_user" | "organization";
export type CredentialUsage = "user" | "service";
export interface CredentialOwner {
  scope: CredentialScope;
  owner_id?: string;
  usage: CredentialUsage;
}
export interface HttpBinding {
  url: string;
  auth: { type: "bearer" } | { type: "header" | "form_field"; name: string };
}
export interface ScopedCredential {
  id: string;
  name: string | null;
  label: string;
  kind: "http" | "mcp_parameter";
  binding: HttpBinding | null;
  mcp_server_url: string | null;
  tool_name: string | null;
  parameter_name: string | null;
  scope: CredentialScope;
  configured: boolean;
  expires_at: string | null;
  revision: number;
  last_used_at: string | null;
  last_used_agent_id: string | null;
  last_used_agent_name: string | null;
  source_connection_id: string | null;
}
export interface CredentialGrant {
  id: string;
  consumer_scope: CredentialScope;
  consumer_id: string | null;
  consumer_label?: string;
  usage: CredentialUsage;
  expires_at: string | null;
}
function ownerQuery(owner: CredentialOwner) {
  return new URLSearchParams(
    Object.entries(owner).filter(
      (entry): entry is [string, string] => typeof entry[1] === "string",
    ),
  ).toString();
}
export const listScopedCredentials = async (owner: CredentialOwner) =>
  (await api.get<ScopedCredential[]>(`/v1/credential-bindings?${ownerQuery(owner)}`)).data;
export const createScopedCredential = async (
  owner: CredentialOwner,
  input: {
    name: string;
    label: string;
    binding?: HttpBinding;
    mcp?: { server_name: string; url: string; tool_name: string; parameter_name: string };
  },
) => (await api.post<{ id: string }>("/v1/credential-bindings", { ...owner, ...input })).data;
export async function saveCredentialValue(
  id: string,
  value: string,
  revision: number,
  expiresAt?: string,
) {
  await api.put(`/v1/credential-bindings/${id}`, {
    value,
    revision,
    expires_at: expiresAt ?? null,
  });
}
export async function revokeScopedCredential(id: string, revision: number) {
  await api.delete(`/v1/credential-bindings/${id}?revision=${revision}`);
}
export const listCredentialGrants = async (id: string) =>
  (await api.get<CredentialGrant[]>(`/v1/resources/${id}/grants`)).data;
export async function grantCredential(
  id: string,
  consumerId: string | undefined,
  expiresAt?: string,
) {
  await api.post(`/v1/resources/${id}/grants`, {
    consumer_scope: consumerId ? "agent" : "organization",
    consumer_id: consumerId,
    usage: "service",
    expires_at: expiresAt ?? null,
  });
}
export async function revokeCredentialGrant(id: string, grantId: string) {
  await api.delete(`/v1/resources/${id}/grants/${grantId}`);
}
