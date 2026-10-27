"use client";

import { createContext, createElement, useContext, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { OrgContext } from "@/providers/org-provider";
import { api } from "@/lib/api/client";
import { queryKeys } from "@/lib/query-keys";
import type { ResourceConfigResponse } from "@/lib/api/types";
import { usePageChatMode } from "./chat-mode-context";
import { useChatPageRows } from "./page-view-state";

const DurablePageAccess = createContext({ view: false, manage: false });

// The layout owns one policy query; row adapters remain optional on isolated
// pages and fail closed without this provider.
export function DurablePageContextProvider({ children }: { children: ReactNode }) {
  const mode = usePageChatMode();
  const org = useContext(OrgContext)?.currentOrg?.public_id;
  // Global rows require the platform policy, never an inferred tenant-admin role.
  const policy = useQuery({
    queryKey: [...queryKeys.policies.config("durable"), org],
    queryFn: async () => (await api.get<ResourceConfigResponse>("/v1/durable/config")).data,
    enabled: !!mode && !!org,
    staleTime: 5 * 60 * 1000,
  });
  const allowed =
    !!mode && !!org && !policy.error && policy.data?.policies?.["durable.view"] === true;
  return createElement(
    DurablePageAccess.Provider,
    {
      value: {
        view: allowed,
        manage: allowed && policy.data?.policies?.["durable.manage"] === true,
      },
    },
    children,
  );
}

export function useDurablePageRows(
  label: string,
  rows: Parameters<typeof useChatPageRows>[1],
  options: { loading?: boolean; error?: boolean; enabled?: boolean } = {},
) {
  const { view: allowed } = useContext(DurablePageAccess);
  useChatPageRows(label, rows, { ...options, enabled: options.enabled !== false && allowed });
}

export function useDurablePageAccess() {
  return useContext(DurablePageAccess);
}
