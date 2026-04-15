import { HydrationBoundary, dehydrate } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import {
  createServerQueryClient,
  getServerRequestContext,
  prefetchAuthBootstrap,
  seedQueryData,
  serverGetList,
} from "@/lib/server-query";
import type { Capability, Harness } from "@/lib/api/types";
import HarnessesPageClient from "./harnesses-page-client";

export default async function HarnessesPage() {
  const queryClient = createServerQueryClient();
  const requestContext = await getServerRequestContext();
  const { currentOrgId } = await prefetchAuthBootstrap(queryClient, requestContext);

  if (currentOrgId) {
    await Promise.all([
      seedQueryData(queryClient, [...queryKeys.harnesses.list(false), currentOrgId], () =>
        serverGetList<Harness>(requestContext, "/v1/harnesses"),
      ),
      seedQueryData(queryClient, ["capabilities", currentOrgId], () =>
        serverGetList<Capability>(requestContext, "/v1/capabilities"),
      ),
    ]);
  }

  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <HarnessesPageClient />
    </HydrationBoundary>
  );
}
