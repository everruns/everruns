import { api } from "./client";
import type { Session } from "./types";

// Voice calls: the browser sends its WebRTC SDP offer, the server places the
// call with the realtime provider and drives the conversation (the agent
// writes every answer). Settings come from a `voice` channel of the agent.

export interface VoiceCallRequest {
  sdp: string;
  /**
   * Voice channel whose settings the call uses. Must belong to the session's
   * agent. Omit to use the agent's first voice channel, or the defaults.
   */
  channel_id?: string;
  /**
   * Optional realtime provider binding: the prefixed public id of the provider
   * connection that serves the call. Omit to use the org default (or single)
   * realtime provider.
   */
  provider_id?: string;
}

export interface ChannelVoiceCallRequest {
  sdp: string;
  /** Continue an existing session of the channel's agent; omit to start a new one. */
  session_id?: string;
  provider_id?: string;
}

export interface VoiceCallResponse {
  voice_connection_id: string;
  provider_call_id?: string | null;
  provider: string;
  model: string;
  voice: string;
  channel_id?: string | null;
  expires_at: string;
  answer_sdp: string;
}

export interface VoiceSessionResponse {
  session: Session;
  voice: VoiceCallResponse;
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

/** Call an agent's voice channel; starts a new session unless `session_id` is given. */
export async function startChannelVoice(
  agentId: string,
  channelId: string,
  request: ChannelVoiceCallRequest,
): Promise<VoiceSessionResponse> {
  const response = await api.post<VoiceSessionResponse>(
    `/v1/agents/${agentId}/channels/${channelId}/voice/calls`,
    request,
  );
  return response.data;
}
