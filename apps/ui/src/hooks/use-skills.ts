"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getSkills,
  getSkill,
  getSkillContent,
  createSkill,
  updateSkill,
  deleteSkill,
  uploadSkillArchive,
} from "@/lib/api/skills";
import { queryKeys } from "@/lib/query-keys";
import type { CreateSkillRequest, UpdateSkillRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// Skill hooks

export function useSkills() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.skills.list(), org],
    queryFn: () => getSkills(),
    enabled: !!org,
    staleTime: 30000,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useSkill(skillId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.skills.detail(skillId), org],
    queryFn: () => getSkill(skillId),
    enabled: !!org && !!skillId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useSkillContent(skillId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.skills.content(skillId), org],
    queryFn: () => getSkillContent(skillId),
    enabled: !!org && !!skillId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateSkill() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateSkillRequest) => createSkill(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills.all });
    },
  });
}

export function useUploadSkill() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (file: File) => uploadSkillArchive(file),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills.all });
    },
  });
}

export function useUpdateSkill(skillId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateSkillRequest) => updateSkill(skillId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.skills.detail(skillId),
      });
    },
  });
}

export function useDeleteSkill() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (skillId: string) => deleteSkill(skillId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills.all });
    },
  });
}
