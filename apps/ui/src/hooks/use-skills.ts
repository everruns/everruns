"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { getSkillContent, skillsCrudApi, uploadSkillArchive } from "@/lib/api/skills";
import type { CreateSkillRequest, UpdateSkillRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks, useOrgScopedQuery } from "./create-crud-hooks";

// Skill hooks

const skillCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof skillsCrudApi.get>>,
  CreateSkillRequest,
  UpdateSkillRequest
>({
  api: skillsCrudApi,
  queryKeys: queryKeys.skills,
  staleTime: 30000,
});

export const useSkills = skillCrudHooks.useList;
export const useSkill = skillCrudHooks.useDetail;
export const useCreateSkill = skillCrudHooks.useCreate;
export const useDeleteSkill = skillCrudHooks.useDelete;
export const useDestroySkill = skillCrudHooks.useDestroy;

export function useUpdateSkill(skillId: string) {
  const mutation = skillCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (request: UpdateSkillRequest, options?: Parameters<typeof mutation.mutate>[1]) =>
      mutation.mutate({ id: skillId, request }, options),
    mutateAsync: (
      request: UpdateSkillRequest,
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: skillId, request }, options),
  };
}

export function useSkillContent(skillId: string) {
  return useOrgScopedQuery({
    queryKey: queryKeys.skills.content(skillId),
    queryFn: () => getSkillContent(skillId),
    enabled: !!skillId,
  });
}

export function useUploadSkill() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: uploadSkillArchive,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills.all });
    },
  });
}
