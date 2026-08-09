import type { Metadata } from "next";
import { HydrationBoundary, dehydrate } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import { formatPageTitle } from "@/lib/page-title";

export const metadata: Metadata = {
  title: formatPageTitle("Sessions"),
};
import {
  createServerQueryClient,
  getServerRequestContext,
  prefetchAuthBootstrap,
  seedQueryData,
  serverGet,
  serverGetList,
  serverGetPaginated,
} from "@/lib/server-query";
import type { Agent, Organization, Session, SessionFacetsResponse } from "@/lib/api/types";
import { SESSIONS_PAGE_SIZE } from "@/lib/session-filters";
import SessionsPageClient from "./sessions-page-client";

/**
 * Seeding is deliberately limited to the unfiltered first page: that is the
 * only filter set whose cache key the server can know without duplicating the
 * client's URL parsing. Any other filter set simply fetches on the client.
 */
export default async function SessionsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const queryClient = createServerQueryClient();
  const requestContext = await getServerRequestContext();
  const { currentOrgId } = await prefetchAuthBootstrap(queryClient, requestContext);
  const isDefaultView = Object.keys(await searchParams).length === 0;

  if (currentOrgId) {
    await Promise.all([
      seedQueryData(queryClient, [...queryKeys.agents.list(true), currentOrgId], () =>
        serverGetList<Agent>(requestContext, "/v1/agents?include_archived=true"),
      ),
      seedQueryData(queryClient, queryKeys.organizations.detail(currentOrgId), () =>
        serverGet<Organization>(requestContext, `/v1/orgs/${currentOrgId}`),
      ),
      ...(isDefaultView
        ? [
            seedQueryData(
              queryClient,
              queryKeys.sessions.filtered(currentOrgId, "", 0, SESSIONS_PAGE_SIZE),
              () =>
                serverGetPaginated<Session>(
                  requestContext,
                  `/v1/sessions?offset=0&limit=${SESSIONS_PAGE_SIZE}`,
                ),
            ),
            seedQueryData(queryClient, queryKeys.sessions.facets(currentOrgId, ""), () =>
              serverGet<SessionFacetsResponse>(requestContext, "/v1/sessions/facets"),
            ),
          ]
        : []),
    ]);
  }

  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <SessionsPageClient />
    </HydrationBoundary>
  );
}
