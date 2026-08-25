import { api } from "./client";
import type { Session } from "./types";

export interface VoiceCallRequest {
  sdp: string;
  model?: string;
  voice?: string;
  reasoning_effort?: string;
  instructions?: string;
  /**
   * Optional realtime provider binding: the prefixed public id of the provider
   * connection to route this voice connection through. Lets an org with more
   * than one realtime-capable provider choose which one serves the session.
   * Omit to use the org default (or single) realtime provider.
   */
  provider_id?: string;
}

export interface VoiceCallResponse {
  voice_connection_id: string;
  provider_call_id?: string | null;
  provider: string;
  model: string;
  voice: string;
  reasoning_effort: string;
  expires_at: string;
  answer_sdp: string;
}

export interface VoiceSessionResponse<T> {
  session: Session;
  voice: T;
}

export async function startSessionVoice(
  sessionId: string,
  request: VoiceCallRequest,
): Promise<VoiceCallResponse> {
  const response = await api.post<VoiceCallResponse>(
    `/v1/sessions/${sessionId}/voice/calls`,
    request,
  );
  return response.data;
}

export async function endSessionVoice(
  sessionId: string,
  voiceConnectionId: string,
  reason?: string,
): Promise<void> {
  await api.post(`/v1/sessions/${sessionId}/voice/${voiceConnectionId}/end`, {
    reason,
  });
}

export async function startAgentVoice(
  agentId: string,
  request: VoiceCallRequest,
): Promise<VoiceSessionResponse<VoiceCallResponse>> {
  const response = await api.post<VoiceSessionResponse<VoiceCallResponse>>(
    `/v1/agents/${agentId}/voice/sessions`,
    request,
  );
  return response.data;
}
