/** Preserve the activity snapshot's bounds across navigation and pagination. */
export function agentFailureHref(agentId: string, before: string, after?: string): string {
  const params = new URLSearchParams({
    tab: "sessions",
    failures: "true",
    after: after ?? new Date(new Date(before).getTime() - 24 * 60 * 60 * 1000).toISOString(),
    before,
  });
  return `/agents/${agentId}?${params}`;
}

export function failureTraceHref(sessionId: string, sequence: number): string {
  return `/sessions/${sessionId}/trace?failure=${sequence}`;
}
