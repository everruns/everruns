"use client";

// Every Sandbox in the organization, across providers and states.
//
// Two views over the same filters: a table for "what exists and in what
// state", and a timeline for "when was each one running". Filters live in the
// URL, so a filtered view (for example Needs attention) can be bookmarked and
// shared.

import { useCallback, useDeferredValue, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Container } from "lucide-react";
import { usePageTitle } from "@/hooks";
import { useSandboxTargets } from "@/hooks/use-sandbox-templates";
import {
  useSandboxFleet,
  useSandboxFleetStats,
  useSandboxTimeline,
} from "@/hooks/use-sandbox-fleet";
import type { SandboxFleetFilters } from "@/lib/api/sandboxes";
import {
  EmptyState,
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import { Button } from "@/components/ui/button";
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
const WINDOWS = { "24h": 24 * 3_600_000, "7d": 7 * 86_400_000 } as const;
type WindowKey = keyof typeof WINDOWS;

export default function SandboxesPageClient() {
  usePageTitle("Sandboxes");
  const router = useRouter();
  const searchParams = useSearchParams();

  const view = searchParams.get("view") === "timeline" ? "timeline" : "table";
  const state = searchParams.get("state") ?? "live";
  const provider = searchParams.get("provider") ?? "all";
  const needsAttention = searchParams.get("attention") === "1";
  const windowKey: WindowKey =
    searchParams.get("window") === "7d" ? "7d" : "24h";
  const [search, setSearch] = useState(searchParams.get("q") ?? "");
  const [page, setPage] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(
    searchParams.get("sandbox"),
  );

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

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Sandboxes" }]} />
      <PageMasthead
        icon={<Container />}
        title="Sandboxes"
        description="Every Sandbox your Sessions ran in, across providers: running, paused, lost and deleted."
      />

      <PageControlStrip>
        <SandboxFleetSummary
          stats={stats.data}
          state={state}
          needsAttention={needsAttention}
          onState={(next) =>
            update({ state: next === "live" ? null : next, attention: null })
          }
          onNeedsAttention={(value) =>
            update({ attention: value ? "1" : null })
          }
        />
      </PageControlStrip>

      <div className="flex flex-col gap-4">
        <SectionTabs
          value={view}
          onValueChange={(next) =>
            update({ view: next === "timeline" ? "timeline" : null })
          }
          items={[
            { value: "table", label: "Table", count: total },
            { value: "timeline", label: "Timeline" },
          ]}
        />

        <div className="flex flex-wrap items-center gap-3">
          <SearchInput
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              setPage(0);
            }}
            placeholder="Search by session, agent, provider or provider id"
            className="min-w-64 flex-1"
          />
          <Select
            value={provider}
            onValueChange={(value) =>
              update({ provider: value === "all" ? null : String(value) })
            }
          >
            <SelectTrigger className="w-44" aria-label="Filter by provider">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All providers</SelectItem>
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
              onValueChange={(value) =>
                update({ window: value === "7d" ? "7d" : null })
              }
            >
              <SelectTrigger className="w-36" aria-label="Time window">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="24h">Last 24 hours</SelectItem>
                <SelectItem value="7d">Last 7 days</SelectItem>
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
              icon={<Container />}
              title="Sandboxes could not be loaded"
              description={
                fleet.error instanceof Error ? fleet.error.message : undefined
              }
            />
          ) : items.length === 0 ? (
            <EmptyState
              icon={<Container />}
              title={
                needsAttention
                  ? "Nothing needs attention"
                  : "No Sandboxes match"
              }
              description={
                needsAttention
                  ? "No Sandbox is lost, failed, idle while running, or stuck in cleanup."
                  : "Sessions get a Sandbox when their Agent or harness uses a Sandbox Template with a provider such as Daytona or Modal. In-process sandboxes are not listed."
              }
            />
          ) : (
            <div className="flex flex-col gap-3">
              <SandboxFleetTable
                items={items}
                selectedId={selectedId}
                onSelect={select}
              />
              {total > PAGE_SIZE ? (
                <div className="flex items-center justify-between text-sm text-muted-foreground">
                  <span className="tabular-nums">
                    {page * PAGE_SIZE + 1} to{" "}
                    {Math.min(total, (page + 1) * PAGE_SIZE)} of {total}
                  </span>
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={page === 0}
                      onClick={() => setPage((p) => p - 1)}
                    >
                      Previous
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={(page + 1) * PAGE_SIZE >= total}
                      onClick={() => setPage((p) => p + 1)}
                    >
                      Next
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
            icon={<Container />}
            title="The timeline could not be loaded"
            description={
              timeline.error instanceof Error
                ? timeline.error.message
                : undefined
            }
          />
        ) : !timeline.data || timeline.data.lanes.length === 0 ? (
          <EmptyState
            icon={<Container />}
            title="No Sandbox ran in this window"
            description="Pick a longer window or clear the filters."
          />
        ) : (
          <SandboxTimelineChart timeline={timeline.data} onSelect={select} />
        )}
      </div>

      <SandboxDetailDrawer
        sandboxId={selectedId}
        onClose={() => select(null)}
      />
    </PageContainer>
  );
}
