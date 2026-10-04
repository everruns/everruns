import { api } from "./client";

export const CHATGPT_USAGE_URL = "https://chatgpt.com/settings/usage";
export type ChatGptConnection = {
  status: "disconnected" | "connecting" | "connected" | "scope_required" | "error";
  email: string | null;
  host_id: string;
  error: string | null;
  owner_user_id?: string | null;
  registration?: { client_id: string; subject: string; email: string | null } | null;
};
const path = (id: string) => `/v1/providers/${id}/chatgpt`;
export async function getChatGptConnection(id: string): Promise<ChatGptConnection> {
  return (await api.get<ChatGptConnection>(path(id))).data;
}
export async function startChatGptLogin(id: string): Promise<{ authorize_url: string }> {
  return (await api.post<{ authorize_url: string }>(`${path(id)}/login`)).data;
}
export async function disconnectChatGpt(id: string): Promise<void> {
  await api.delete(path(id));
}
export async function importChatGptConnection(id: string, connection: unknown): Promise<void> {
  await api.post(`${path(id)}/import`, connection);
}
