// Commands API functions

import { api } from "./client";
import type { CommandResult, CommandsResponse, ExecuteCommandRequest } from "./types";

export async function getSessionCommands(sessionId: string): Promise<CommandsResponse> {
  const response = await api.get<CommandsResponse>(`/v1/sessions/${sessionId}/commands`);
  return response.data;
}

export async function executeSessionCommand(
  sessionId: string,
  req: ExecuteCommandRequest,
): Promise<CommandResult> {
  const response = await api.post<CommandResult>(`/v1/sessions/${sessionId}/commands/execute`, req);
  return response.data;
}
