"use client";
import { SegmentedControl } from "@/components/ui/segmented-control";

import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

// Sandboxes tab: every Sandbox in the organization, across providers and
// states. The page header and tabs come from the shell (sandboxes-shell.tsx).
//
// Two views over the same filters: a table for "what exists and in what
// state", and a timeline for "when was each one running". The view is a
// toggle beside the filters, not a second row of tabs under the page's tabs.
// Filters live in the URL, so a filtered view (for example Needs attention)
// can be bookmarked and shared.

import { SandboxIcon } from "@/components/icons/facet-icons";
import { useCallback, useDeferredValue, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { ChartGantt, Rows3 } from "lucide-react";

import { usePageTitle } from "@/hooks";
import { useSandboxTargets } from "@/hooks/use-sandbox-templates";
import {
  useSandboxFleet,
  useSandboxFleetStats,
  useSandboxTimeline,
} from "@/hooks/use-sandbox-fleet";
import type { SandboxFleetFilters } from "@/lib/api/sandboxes";
import { EmptyState, PageControlStrip } from "@/components/layout";
import Link from "next/link";
import { Button, buttonVariants } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { SandboxDetailDrawer } from "@/components/sandboxes/sandbox-detail-drawer";
import { providerLabel } from "@/components/sandboxes/sandbox-display";
import { SandboxFleetSummary } from "@/components/sandboxes/sandbox-fleet-summary";
import { SandboxFleetTable } from "@/components/sandboxes/sandbox-fleet-table";
import { SandboxTimelineChart } from "@/components/sandboxes/sandbox-timeline";

const PAGE_SIZE = 50;
const UNFILTERED: SandboxFleetFilters = {
  state: "all",
  provider: "all",
  needsAttention: false,
  search: "",
};
const WINDOWS = { "24h": 24 * 3_600_000, "7d": 7 * 86_400_000 } as const;
type WindowKey = keyof typeof WINDOWS;

export function SandboxFleetTab() {
  const ui = useUiTranslations();
  usePageTitle(ui("Sandboxes"));
  const router = useRouter();
  const searchParams = useSearchParams();

  const view = searchParams.get("view") === "timeline" ? "timeline" : "table";
  const state = searchParams.get("state") ?? "live";
  const provider = searchParams.get("provider") ?? "all";
  const needsAttention = searchParams.get("attention") === "1";
  const windowKey: WindowKey = searchParams.get("window") === "7d" ? "7d" : "24h";
  const [search, setSearch] = useState(searchParams.get("q") ?? "");
  const [page, setPage] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(searchParams.get("sandbox"));

  useChatPageFilters({
    search,
    status: state,
    provider_id: provider,
    filter: needsAttention ? "needs_attention" : null,
    range: windowKey,
    tab: view,
    page,
    limit: PAGE_SIZE,
    selected_id: selectedId,
  });

  const update = useCallback(
    (changes: Record<string, string | null>) => {
      const params = new URLSearchParams(searchParams.toString());
      for (const [key, value] of Object.entries(changes)) {
        if (value === null) params.delete(key);
        else params.set(key, value);
      }
      const query = params.toString();
      router.replace(query ? `/sandboxes?${query}` : "/sandboxes", {
        scroll: false,
      });
      setPage(0);
    },
    [router, searchParams],
  );

  const deferredSearch = useDeferredValue(search);
  const filters: SandboxFleetFilters = useMemo(
    () => ({
      state: needsAttention ? "all" : state,
      provider,
      needsAttention,
      search: deferredSearch,
    }),
    [needsAttention, state, provider, deferredSearch],
  );

  // A window anchored to when the view or window last changed, so the query
  // key is stable between renders.
  const timelineWindow = useMemo(() => {
    const to = new Date();
    return {
      from: new Date(to.getTime() - WINDOWS[windowKey]).toISOString(),
      to: to.toISOString(),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [windowKey, view]);

  const fleet = useSandboxFleet(filters, {
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
  });
  const stats = useSandboxFleetStats(filters);
  // Unfiltered roll-up: tells "this org has never had a Sandbox" apart from "the filters
  // hide every Sandbox". A fresh org gets one teaching empty state instead of a dashboard
  // of zero tiles, zero chips and filters with nothing to filter.
  const everything = useSandboxFleetStats(UNFILTERED);
  const fleetIsEmpty =
    everything.data !== undefined &&
    everything.data.by_state.reduce((sum, entry) => sum + entry.count, 0) === 0;
  const timeline = useSandboxTimeline(
    {
      ...filters,
      state: needsAttention ? "all" : state === "live" ? "all" : state,
    },
    timelineWindow,
    view === "timeline",
  );
  const targets = useSandboxTargets();

  const providers = useMemo(() => {
    const ids = new Set<string>();
    for (const target of targets.data?.items ?? []) {
      if (target.provider && target.kind !== "vfs" && target.kind !== "host")
        ids.add(target.provider);
    }
    for (const entry of stats.data?.live_by_provider ?? []) ids.add(entry.key);
    for (const item of fleet.data?.items ?? []) ids.add(item.provider);
    if (provider !== "all") ids.add(provider);
    return Array.from(ids).sort();
  }, [targets.data, stats.data, fleet.data, provider]);

  const select = (id: string | null) => {
    setSelectedId(id);
    const params = new URLSearchParams(searchParams.toString());
    if (id) params.set("sandbox", id);
    else params.delete("sandbox");
    const query = params.toString();
    router.replace(query ? `/sandboxes?${query}` : "/sandboxes", {
      scroll: false,
    });
  };

  const total = fleet.data?.total ?? 0;
  const items = fleet.data?.items ?? [];

  if (fleetIsEmpty) {
    return (
      <EmptyState
        icon={<SandboxIcon />}
        title={ui("No Sandboxes yet")}
        description={ui(
          "A Session gets a Sandbox when its Agent or harness uses a Sandbox Template with a provider such as Daytona or Modal. Sandboxes appear here once a Session starts one. In-process sandboxes are not listed.",
        )}
        action={
          <Link href="/sandboxes/templates" className={buttonVariants({ variant: "outline" })}>
            {ui("Set up a Sandbox Template")}
          </Link>
        }
      />
    );
  }

  return (
    <>
      <PageControlStrip>
        <SandboxFleetSummary
          stats={stats.data}
          state={state}
          needsAttention={needsAttention}
          onState={(next) => update({ state: next === "live" ? null : next, attention: null })}
          onNeedsAttention={(value) => update({ attention: value ? "1" : null })}
        />
      </PageControlStrip>

      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-3">
          <SegmentedControl
            label={ui("View")}
            value={view}
            onValueChange={(value) => update({ view: value === "timeline" ? "timeline" : null })}
            options={[
              { value: "table", label: ui("Table"), icon: <Rows3 className="size-4" /> },
              { value: "timeline", label: ui("Timeline"), icon: <ChartGantt className="size-4" /> },
            ]}
          />
          <SearchInput
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              setPage(0);
            }}
            placeholder={ui("Search sessions, agents, provider ids")}
            className="min-w-64 flex-1"
          />
          <Select
            value={provider}
            onValueChange={(value) => update({ provider: value === "all" ? null : String(value) })}
          >
            <SelectTrigger className="w-44" aria-label={ui("Filter by provider")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{ui("All providers")}</SelectItem>
              {providers.map((id) => (
                <SelectItem key={id} value={id}>
                  {providerLabel(id)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {view === "timeline" ? (
            <Select
              value={windowKey}
              onValueChange={(value) => update({ window: value === "7d" ? "7d" : null })}
            >
              <SelectTrigger className="w-36" aria-label={ui("Time window")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="24h">{ui("Last 24 hours")}</SelectItem>
                <SelectItem value="7d">{ui("Last 7 days")}</SelectItem>
              </SelectContent>
            </Select>
          ) : null}
        </div>

        {view === "table" ? (
          fleet.isLoading ? (
            <div className="flex flex-col gap-2">
              {Array.from({ length: 6 }, (_, i) => (
                <Skeleton key={i} className="h-9 w-full" />
              ))}
            </div>
          ) : fleet.error ? (
            <EmptyState
              icon={<SandboxIcon />}
              title={ui("Sandboxes could not be loaded")}
              description={fleet.error instanceof Error ? fleet.error.message : undefined}
            />
          ) : items.length === 0 ? (
            <EmptyState
              icon={<SandboxIcon />}
              title={
                needsAttention
                  ? ui("Nothing needs attention")
                  : ui("No Sandboxes match these filters")
              }
              description={
                needsAttention
                  ? ui("No Sandbox is lost, failed, idle while running, or stuck in cleanup.")
                  : ui("Try another state, provider or search.")
              }
            />
          ) : (
            <div className="flex flex-col gap-3">
              <SandboxFleetTable items={items} selectedId={selectedId} onSelect={select} />
              {total > PAGE_SIZE ? (
                <div className="flex items-center justify-between text-sm text-muted-foreground">
                  <span className="tabular-nums">
                    {page * PAGE_SIZE + 1} {ui("to")} {Math.min(total, (page + 1) * PAGE_SIZE)}{" "}
                    {ui("of")} {total}
                  </span>
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={page === 0}
                      onClick={() => setPage((p) => p - 1)}
                    >
                      {ui("Previous")}
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={(page + 1) * PAGE_SIZE >= total}
                      onClick={() => setPage((p) => p + 1)}
                    >
                      {ui("Next")}
                    </Button>
                  </div>
                </div>
              ) : null}
            </div>
          )
        ) : timeline.isLoading ? (
          <Skeleton className="h-64 w-full" />
        ) : timeline.error ? (
          <EmptyState
            icon={<SandboxIcon />}
            title={ui("The timeline could not be loaded")}
            description={timeline.error instanceof Error ? timeline.error.message : undefined}
          />
        ) : !timeline.data || timeline.data.lanes.length === 0 ? (
          <EmptyState
            icon={<SandboxIcon />}
            title={ui("No Sandbox ran in this window")}
            description={ui("Pick a longer window or clear the filters.")}
          />
        ) : (
          <SandboxTimelineChart timeline={timeline.data} onSelect={select} />
        )}
      </div>

      <SandboxDetailDrawer sandboxId={selectedId} onClose={() => select(null)} />
    </>
  );
}
