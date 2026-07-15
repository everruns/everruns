// Session participant API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import type { SessionParticipant, AddSessionParticipantRequest } from "./types";

/** List participants for a session (active and left). */
export async function listSessionParticipants(sessionId: string): Promise<SessionParticipant[]> {
  const response = await api.get<SessionParticipant[]>(`/v1/sessions/${sessionId}/participants`);
  return response.data;
}

/**
 * Add a participant to a session. Only members can be added — the server
 * rejects `role: "host"`. Agent participants require `agent_id`.
 */
export async function addSessionParticipant(
  sessionId: string,
  body: AddSessionParticipantRequest,
): Promise<SessionParticipant> {
  const response = await api.post<SessionParticipant>(
    `/v1/sessions/${sessionId}/participants`,
    body,
  );
  return response.data;
}

/**
 * Remove a participant from a session (sets `left_at`). The server returns 409
 * when attempting to remove the host.
 */
export async function leaveSessionParticipant(
  sessionId: string,
  participantId: string,
): Promise<SessionParticipant> {
  const response = await api.delete<SessionParticipant>(
    `/v1/sessions/${sessionId}/participants/${participantId}`,
  );
  return response.data;
}
