// Organization invitation API functions (EVE-602).
// Routes:
//   GET/POST   /v1/orgs/{org}/invites
//   DELETE     /v1/orgs/{org}/invites/{inviteId}
//   POST       /v1/invites/{token}/accept

import { api } from "./client";
import type { OrgRole } from "./types";

export type InvitationStatus = "pending" | "expired" | "accepted" | "revoked";

/** Outcome of the optional email-delivery attempt at creation time. */
export type EmailDelivery = "sent" | "failed" | "not_configured";

export interface Invitation {
  id: string;
  email: string;
  role: OrgRole;
  status: InvitationStatus;
  invited_by: string;
  created_at: string;
  expires_at: string;
}

/** Create response: invitation plus the one-time invite URL and delivery status. */
export interface CreatedInvitation extends Invitation {
  invite_url: string;
  email_delivery: EmailDelivery;
}

export interface AcceptedInvitation {
  org_id: string;
  role: OrgRole;
}

interface InvitationsListResponse {
  data: Invitation[];
}

export async function listInvites(org: string): Promise<Invitation[]> {
  const response = await api.get<InvitationsListResponse>(`/v1/orgs/${org}/invites`);
  return response.data.data;
}

export async function createInvite(
  org: string,
  email: string,
  role: OrgRole = "member",
): Promise<CreatedInvitation> {
  const response = await api.post<CreatedInvitation>(`/v1/orgs/${org}/invites`, {
    email,
    role,
  });
  return response.data;
}

export async function revokeInvite(org: string, inviteId: string): Promise<void> {
  await api.delete(`/v1/orgs/${org}/invites/${inviteId}`);
}

export async function acceptInvite(token: string): Promise<AcceptedInvitation> {
  const response = await api.post<AcceptedInvitation>(
    `/v1/invites/${encodeURIComponent(token)}/accept`,
  );
  return response.data;
}
