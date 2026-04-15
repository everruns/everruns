import { HydrationBoundary, dehydrate } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import {
  createServerQueryClient,
  getServerRequestContext,
  prefetchAuthBootstrap,
  seedQueryData,
  serverGet,
  serverGetList,
} from "@/lib/server-query";
import type { ResourceConfigResponse, Skill } from "@/lib/api/types";
import SkillsPageClient from "./skills-page-client";

export default async function SkillsPage() {
  const queryClient = createServerQueryClient();
  const requestContext = await getServerRequestContext();
  const { currentOrgId } = await prefetchAuthBootstrap(queryClient, requestContext);

  if (currentOrgId) {
    await Promise.all([
      seedQueryData(queryClient, [...queryKeys.skills.list(false), currentOrgId], () =>
        serverGetList<Skill>(requestContext, "/v1/skills"),
      ),
      seedQueryData(queryClient, [...queryKeys.policies.config("skills"), currentOrgId], () =>
        serverGet<ResourceConfigResponse>(requestContext, "/v1/skills/config"),
      ),
    ]);
  }

  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <SkillsPageClient />
    </HydrationBoundary>
  );
}
