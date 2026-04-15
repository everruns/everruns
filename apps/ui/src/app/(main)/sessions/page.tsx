import { HydrationBoundary, dehydrate } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import {
  createServerQueryClient,
  getServerRequestContext,
  prefetchAuthBootstrap,
  seedQueryData,
  serverGet,
  serverGetList,
  serverGetPaginated,
} from "@/lib/server-query";
import type { Agent, Harness, LlmModelWithProvider, Organization, Session } from "@/lib/api/types";
import SessionsPageClient from "./sessions-page-client";

const PAGE_SIZE = 20;

export default async function SessionsPage() {
  const queryClient = createServerQueryClient();
  const requestContext = await getServerRequestContext();
  const { currentOrgId } = await prefetchAuthBootstrap(queryClient, requestContext);

  if (currentOrgId) {
    await Promise.all([
      seedQueryData(queryClient, [...queryKeys.agents.list(true), currentOrgId], () =>
        serverGetList<Agent>(requestContext, "/v1/agents?include_archived=true"),
      ),
      seedQueryData(queryClient, queryKeys.organizations.detail(currentOrgId), () =>
        serverGet<Organization>(requestContext, `/v1/orgs/${currentOrgId}`),
      ),
      seedQueryData(queryClient, [...queryKeys.harnesses.list(false), currentOrgId], () =>
        serverGetList<Harness>(requestContext, "/v1/harnesses"),
      ),
      seedQueryData(
        queryClient,
        queryKeys.sessions.list(currentOrgId, undefined, 0, PAGE_SIZE),
        () =>
          serverGetPaginated<Session>(requestContext, `/v1/sessions?offset=0&limit=${PAGE_SIZE}`),
      ),
      seedQueryData(queryClient, [...queryKeys.llmModels.list(), currentOrgId], () =>
        serverGetList<LlmModelWithProvider>(requestContext, "/v1/llm-models"),
      ),
    ]);
  }

  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <SessionsPageClient />
    </HydrationBoundary>
  );
}
