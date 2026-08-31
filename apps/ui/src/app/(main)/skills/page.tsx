import type { Metadata } from "next";
import { HydrationBoundary, dehydrate } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import { formatPageTitle } from "@/lib/page-title";

export const metadata: Metadata = {
  title: formatPageTitle("Skills"),
};
import {
  createServerQueryClient,
  getServerRequestContext,
  prefetchAuthBootstrap,
  seedQueryData,
  serverGet,
  serverGetList,
} from "@/lib/server-query";
import type { FeatureFlags, ResourceConfigResponse, Skill } from "@/lib/api/types";
import SkillsPageClient from "./skills-page-client";

export default async function SkillsPage() {
  const queryClient = createServerQueryClient();
  const requestContext = await getServerRequestContext();
  const { currentOrgId } = await prefetchAuthBootstrap(queryClient, requestContext);

  if (currentOrgId) {
    const flags = await seedQueryData(queryClient, ["feature-flags", "org", currentOrgId], () =>
      serverGet<FeatureFlags>(requestContext, `/v1/orgs/${currentOrgId}/feature-flags`),
    );

    if (flags?.skills) {
      await Promise.all([
        seedQueryData(queryClient, [...queryKeys.skills.list(false), currentOrgId], () =>
          serverGetList<Skill>(requestContext, "/v1/skills"),
        ),
        seedQueryData(queryClient, [...queryKeys.policies.config("skills"), currentOrgId], () =>
          serverGet<ResourceConfigResponse>(requestContext, "/v1/skills/config"),
        ),
      ]);
    }
  }

  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <SkillsPageClient />
    </HydrationBoundary>
  );
}
