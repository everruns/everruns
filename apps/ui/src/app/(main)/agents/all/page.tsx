import { HydrationBoundary, dehydrate } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import {
  createServerQueryClient,
  getServerRequestContext,
  prefetchAuthBootstrap,
  seedQueryData,
  serverGetList,
} from "@/lib/server-query";
import type { Agent, Capability } from "@/lib/api/types";
import AgentsAllPageClient from "./agents-all-page-client";

export default async function AllAgentsPage() {
  const queryClient = createServerQueryClient();
  const requestContext = await getServerRequestContext();
  const { currentOrgId } = await prefetchAuthBootstrap(queryClient, requestContext);

  if (currentOrgId) {
    await Promise.all([
      seedQueryData(queryClient, [...queryKeys.agents.list(false), currentOrgId], () =>
        serverGetList<Agent>(requestContext, "/v1/agents"),
      ),
      seedQueryData(queryClient, ["capabilities", currentOrgId], () =>
        serverGetList<Capability>(requestContext, "/v1/capabilities"),
      ),
    ]);
  }

  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <AgentsAllPageClient />
    </HydrationBoundary>
  );
}
