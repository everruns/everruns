"use client";

import { useMemo, useRef, useLayoutEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { isPageQuery, summarizePageData } from "@/lib/chat-page-view";
import type { ChatPageContext } from "./chat-mode-context";
import { assertDraftSize, type PageDraftAdapter, type PageDraftSnapshot } from "./page-draft";

export function usePageViewAdapter(
  page: ChatPageContext,
  org?: string,
  readUI?: () => unknown,
): PageDraftAdapter {
  const client = useQueryClient();
  const current = useRef({ page, readUI });
  useLayoutEffect(() => {
    current.current = { page, readUI };
  }, [page, readUI]);
  return useMemo(() => {
    const route = page.route;
    const binding = crypto.randomUUID();
    let signature = "";
    let revision = crypto.randomUUID();
    function snapshot(): PageDraftSnapshot {
      const context = current.current.page;
      const budget = { nodes: 0, characters: 0, truncated: false };
      const active = org
        ? client
            .getQueryCache()
            .getAll()
            .filter((query) => query.isActive() && isPageQuery(query.queryKey, context, org))
            .slice(0, 12)
        : [];
      const resources = active
        .filter((query) => query.state.status === "success")
        .map((query) => ({
          resource: query.queryKey[0],
          refreshing: query.state.fetchStatus === "fetching",
          data: summarizePageData(query.state.data, 0, budget),
        }));
      const values = {
        page: context,
        organization_id: org,
        resources,
        load_state: active.map((query) => ({
          resource: query.queryKey[0],
          status: query.state.status,
          refreshing: query.state.fetchStatus === "fetching",
        })),
        view: current.current.readUI?.(),
        summary_truncated: budget.truncated,
        summary_limits: { items: 50, text: 512, depth: 6, characters: 16384, nodes: 2000 },
        editing:
          "This page-view summary is read-only. Use the active form's writable flag and patch_schema for draft edits. Immediate actions and credentials retain their normal controls.",
      };
      const next = JSON.stringify(values);
      if (next !== signature) {
        signature = next;
        revision = crypto.randomUUID();
      }
      const result = {
        binding,
        revision,
        entity_id: context.entity_id ?? context.route,
        entity_type: context.entity_type,
        values,
        writable: false,
        dirty: false,
      };
      assertDraftSize(result);
      return result;
    }
    return {
      binding,
      // Sending only publishes the binding. Summaries are assembled exclusively
      // for an explicit read tool, so a large list cannot block the composer.
      describe: () => ({ binding, revision, dirty: false, writable: false }),
      read: (token: string) => {
        if (token !== binding || current.current.page.route !== route)
          throw new Error("The page changed. Send a new message from the current page.");
        return structuredClone(snapshot());
      },
      patch: () => {
        throw new Error(
          "This page has no active editable form. Use its existing controls; no changes were applied.",
        );
      },
    };
  }, [page.route, org, client]);
}
