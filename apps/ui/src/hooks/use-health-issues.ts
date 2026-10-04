"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  listHealthIssues,
  getHealthIssue,
  checkHealthIssue,
  snoozeHealthIssue,
} from "@/lib/api/health-issues";
import { ApiError } from "@/lib/api/client";
import { useOrg } from "@/providers/org-provider";

export function useHealthIssues(channelId?: string, offset = 0, enabled = true) {
  const { currentOrg } = useOrg();
  const query = useQuery({
    queryKey: ["health-issues", currentOrg?.public_id, channelId, offset],
    queryFn: () => listHealthIssues(channelId, offset),
    enabled: enabled && !!currentOrg,
    refetchInterval: 30_000,
    retry: false,
  });
  // Cached evidence must disappear when resource access is revoked or removed.
  const unavailable =
    query.error instanceof ApiError && [401, 403, 404].includes(query.error.status);
  return { ...query, data: unavailable ? undefined : query.data };
}
export function useHealthIssue(id: string | null) {
  const { currentOrg } = useOrg();
  const query = useQuery({
    queryKey: ["health-issue", currentOrg?.public_id, id],
    queryFn: () => getHealthIssue(id!),
    enabled: !!id && !!currentOrg,
    refetchInterval: 30_000,
    retry: false,
  });
  // Cached evidence must disappear when resource access is revoked or removed.
  const unavailable =
    query.error instanceof ApiError && [401, 403, 404].includes(query.error.status);
  return { ...query, data: unavailable ? undefined : query.data };
}
export function useHealthIssueAction(action: "check" | "snooze") {
  const client = useQueryClient();
  return useMutation({
    mutationFn: action === "check" ? checkHealthIssue : snoozeHealthIssue,
    onSuccess: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey: ["health-issues"] }),
        client.invalidateQueries({ queryKey: ["health-issue"] }),
        client.invalidateQueries({ queryKey: ["notifications"] }),
      ]);
    },
  });
}
