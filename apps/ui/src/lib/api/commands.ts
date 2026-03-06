// Commands API functions

import { api } from "./client";
import type { CommandsResponse } from "./types";

export async function getSessionCommands(sessionId: string): Promise<CommandsResponse> {
  const response = await api.get<CommandsResponse>(`/v1/sessions/${sessionId}/commands`);
  return response.data;
}
