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
export async function dismissHealthIssue(id: string, note: string) {
  return (await api.post<HealthIssue>(`/v1/health-issues/${id}/dismiss`, { note })).data;
}
export interface HealthIssueFixOutcome {
  status: "done" | "consent";
  consent_url?: string | null;
  returns_automatically: boolean;
  issue: HealthIssue;
}
// Runs one of the fixers the server declared on the issue. The server builds
// any consent URL; `safeConsentUrl` checks it again before the browser leaves.
export async function runHealthIssueFixer(id: string, kind: string) {
  return (await api.post<HealthIssueFixOutcome>(`/v1/health-issues/${id}/fix`, { kind })).data;
}

/** The provider pages a fixer may send the browser to, and nothing else. */
export function safeConsentUrl(raw: string | null | undefined): URL | null {
  if (!raw) return null;
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" || url.username || url.password || url.port) return null;
  if (url.origin === "https://slack.com" && url.pathname === "/oauth/v2/authorize") return url;
  if (
    url.origin === "https://api.slack.com" &&
    /^\/apps\/A[A-Z0-9]+\/install-on-team$/.test(url.pathname) &&
    !url.search
  )
    return url;
  return null;
}
