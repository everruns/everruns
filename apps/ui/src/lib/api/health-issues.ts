import { api } from "./client";
import type { HealthIssue, HealthIssueList } from "./types";

export async function listHealthIssues(channelId?: string, offset = 0, limit = 20) {
  const params = new URLSearchParams({ offset: String(offset), limit: String(limit) });
  if (channelId) params.set("channel_id", channelId);
  return (await api.get<HealthIssueList>(`/v1/health-issues?${params}`)).data;
}
export async function getHealthIssue(id: string) {
  return (await api.get<HealthIssue>(`/v1/health-issues/${id}`)).data;
}
export async function checkHealthIssue(id: string) {
  return (await api.post<HealthIssue>(`/v1/health-issues/${id}/check`)).data;
}
export async function snoozeHealthIssue(id: string) {
  return (await api.post<HealthIssue>(`/v1/health-issues/${id}/snooze`)).data;
}
