/**
 * The organization every API request is answered in.
 *
 * Org selection used to travel only in the `everruns_org` cookie, which the
 * app sets with a fire-and-forget `POST /v1/users/me/switch-org`. The client
 * commits the new org to React state immediately, so every request issued
 * before that POST lands is answered in the *previous* org while the client
 * files the result under the new one — and two queries straddling the switch
 * come back from different orgs (EVE-984 made the server honour an explicit
 * header for exactly this reason).
 *
 * So the active org is also carried per request in `X-Org-Id`, which cannot
 * race: the header on the wire is the org the caller asked for. The cookie
 * stays in sync for transports that cannot set headers (SSE, downloads that
 * navigate the browser).
 *
 * This is a module-level holder rather than context because the API layer is
 * plain functions called from query functions, not components — and it must be
 * readable synchronously, before any effect runs.
 */

export const ORG_HEADER = "X-Org-Id";

let activeOrgId: string | null = null;

/** Record the org subsequent requests belong to. Called by `OrgProvider` the
 *  moment an org is chosen, before the state commit renders any consumer. */
export function setActiveOrgId(orgId: string | null): void {
  activeOrgId = orgId;
}

export function getActiveOrgId(): string | null {
  return activeOrgId;
}

/** Merge `X-Org-Id` into request headers. An explicit header from the caller
 *  wins; with no active org the request falls back to the cookie. */
export function withOrgHeader<T extends Record<string, string>>(
  headers?: T,
): Record<string, string> {
  const merged: Record<string, string> = { ...(headers ?? {}) };
  const hasExplicit = Object.keys(merged).some((key) => key.toLowerCase() === "x-org-id");
  if (activeOrgId && !hasExplicit) merged[ORG_HEADER] = activeOrgId;
  return merged;
}
