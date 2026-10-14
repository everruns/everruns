// Org admin links to the shared Everruns GitHub App (D3 in
// knowledge/integrations/github-model.md). Linking goes through GitHub in the
// browser, so it is a URL to navigate to, not an API call.

import { api, getBackendUrl } from "./client";

export interface EverrunsAppLink {
  id: string;
  installation_id: number;
  account: string;
  account_type: string | null;
  repository_selection: string | null;
  agent_count: number;
  linked_at: string;
}

export interface EverrunsAppLinks {
  configured: boolean;
  app_slug: string | null;
  installations: EverrunsAppLink[];
}

function base(org: string): string {
  return `/v1/orgs/${encodeURIComponent(org)}/github/everruns-app`;
}

export async function listEverrunsAppLinks(org: string): Promise<EverrunsAppLinks> {
  return (await api.get<EverrunsAppLinks>(base(org))).data;
}

export async function unlinkEverrunsApp(org: string, connectionId: string): Promise<void> {
  await api.delete(`${base(org)}/${encodeURIComponent(connectionId)}`);
}

/** Where the browser goes to link an installation; GitHub returns to `returnTo`. */
export function everrunsAppLinkUrl(org: string, returnTo: string): string {
  return `${getBackendUrl()}${base(org)}/authorize?return_to=${encodeURIComponent(returnTo)}`;
}
