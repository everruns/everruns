"use client";

/**
 * Sessions — the operational list (EVE-853).
 *
 * This page answers "what ran". It is read-only on purpose: authoring a run
 * belongs where the run is composed (`/chats/new` for an interactive thread,
 * the agent page for an agent run), not on the surface you open when something
 * has gone wrong. The old "New session" dialog moved out for that reason; both
 * entry points already existed, so nothing was lost.
 *
 * The whole filter set lives in the URL. That is what makes a saved view just a
 * link, and it keeps three things — the rows, the facet counts, and the address
 * bar — describing one population. Counts come from `GET /v1/sessions/facets`,
 * never from the rows on screen.
 */

import { useCallback, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Activity, ChevronLeft, ChevronRight, Download, MessageSquare } from "lucide-react";
import { Badge } from "@/components/ui/badge";
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
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  SectionTabs,
  EmptyState,
} from "@/components/layout";
import { SessionRow } from "@/components/session/session-row";
import { SessionFacetRail } from "@/components/session/session-facet-rail";
import { useAgents, useFilteredSessions, useSessionFacets, usePageTitle } from "@/hooks";
import { useSessionViews } from "@/hooks/use-session-views";
import { useOptionalNotificationsContext } from "@/providers/notifications-provider";
import { downloadSessionsCsv } from "@/lib/session-list-export";
import { formatDurationCompact, formatTokens, pluralize } from "@/lib/formatting";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { shortenId } from "@/lib/utils";
import {
  EMPTY_SESSION_FILTERS,
  SESSIONS_PAGE_SIZE,
  TIME_WINDOWS,
  TIME_WINDOW_LABELS,
  hasActiveFilters,
  parseSessionFilters,
  serializeSessionFilters,
  toQueryParams,
  toggleAgentFilter,
  toggleFilterValue,
  withFilterChange,
  type SessionFilters,
  type SessionTimeWindow,
} from "@/lib/session-filters";
import type { SavedSessionView } from "@/lib/session-views";

/**
 * The segmented control is a shortcut into the same `status` facet, not a
 * separate filter. "All" clears the dimension; anything else selects exactly
 * one value, so the control can always show a single active segment.
 */
const STATUS_SEGMENTS = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "failed", label: "Failed" },
  { value: "completed", label: "Completed" },
];

function segmentFor(filters: SessionFilters): string {
  return filters.status.length === 1 ? filters.status[0] : "all";
}

export default function SessionsPageClient() {
  usePageTitle("Sessions");
  const router = useRouter();
  const searchParams = useSearchParams();
  const notificationsContext = useOptionalNotificationsContext();

  const filters = useMemo(
    () => parseSessionFilters(new URLSearchParams(searchParams.toString())),
    [searchParams],
  );

  // Search is typed locally and committed to the URL on submit or blur. Pushing
  // every keystroke into the URL would make Back walk letter by letter.
  const [searchDraft, setSearchDraft] = useState(filters.search);
  const [committedSearch, setCommittedSearch] = useState(filters.search);
  const [exporting, setExporting] = useState(false);

  // The URL can change without the field being touched — selecting a saved view,
  // or clearing filters. Re-sync the draft when it does, so the box never shows
  // a term the list is not filtered by.
  if (filters.search !== committedSearch) {
    setCommittedSearch(filters.search);
    setSearchDraft(filters.search);
  }

  const applyFilters = useCallback(
    (next: SessionFilters) => {
      const query = serializeSessionFilters(next);
      router.replace(query ? `/sessions?${query}` : "/sessions", { scroll: false });
    },
    [router],
  );

  const { data: agents } = useAgents({ includeArchived: true });
  const {
    data: sessionsResponse,
    isLoading: sessionsLoading,
    error: sessionsError,
    refetch: refetchSessions,
  } = useFilteredSessions(filters);
  const { data: facets, error: facetsError } = useSessionFacets(filters);
  const views = useSessionViews();

  const agentNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const agent of agents ?? []) {
      map.set(agent.id, getDisplayName(agent) || shortenId(agent.id));
    }
    return map;
  }, [agents]);

  const agentLabel = useCallback(
    (agentId: string) => agentNames.get(agentId) ?? shortenId(agentId),
    [agentNames],
  );

  const sessions = sessionsResponse?.data ?? [];
  // The list total is the filtered total; the facet total is the same number
  // computed independently. Prefer the list's own, so pagination arithmetic can
  // never disagree with the rows it is describing.
  const total = sessionsResponse?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / SESSIONS_PAGE_SIZE));
  const offset = filters.page * SESSIONS_PAGE_SIZE;
  const filtered = hasActiveFilters(filters);

  const handleExport = useCallback(async () => {
    setExporting(true);
    try {
      const result = await downloadSessionsCsv(toQueryParams(filters), agentLabel);
      notificationsContext?.notify?.({
        title: "Sessions exported",
        body: result.truncated
          ? `${result.rowsExported} rows exported (capped). Narrow the filters to export the rest.`
          : `${result.rowsExported} ${pluralize(result.rowsExported, "row")} exported.`,
      });
    } catch (error) {
      console.error("Failed to export sessions:", error);
      notificationsContext?.notify?.({
        title: "Export failed",
        body: "Could not export the filtered sessions. Try again.",
      });
    } finally {
      setExporting(false);
    }
  }, [agentLabel, filters, notificationsContext]);

  const handleSelectView = useCallback(
    (view: SavedSessionView) => {
      applyFilters(parseSessionFilters(new URLSearchParams(view.query)));
    },
    [applyFilters],
  );

  const handleSaveView = useCallback(
    (name: string) => {
      void views.saveView(name, serializeSessionFilters({ ...filters, page: 0 }));
    },
    [filters, views],
  );

  const metrics = facets && [
    { label: "Active now", value: String(facets.active_now) },
    { label: "Failed today", value: String(facets.failed_today) },
    { label: "p95 duration", value: formatDurationCompact(facets.p95_duration_ms) },
    { label: "Tokens today", value: formatTokens(facets.tokens_today) },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Sessions" }]} />

      <PageMasthead
        icon={<MessageSquare />}
        title="Sessions"
        badges={
          <>
            <Badge variant="outline" className="font-mono">
              {total}
            </Badge>
            <Badge variant="secondary">read-only</Badge>
          </>
        }
        description="Every run across your agents — chats, schedules, channels and API calls."
        meta={
          metrics ? (
            <>
              {metrics.map((metric) => (
                <span key={metric.label}>
                  {metric.label} <span className="text-foreground">{metric.value}</span>
                </span>
              ))}
            </>
          ) : (
            <span>Loading counters…</span>
          )
        }
        actions={
          <Button variant="outline" onClick={handleExport} disabled={exporting || total === 0}>
            <Download className="size-4" />
            {exporting ? "Exporting…" : "Export"}
          </Button>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <form
          className="min-w-[12rem] flex-[1_1_16rem]"
          onSubmit={(event) => {
            event.preventDefault();
            applyFilters(withFilterChange(filters, { search: searchDraft.trim() }));
          }}
        >
          <SearchInput
            value={searchDraft}
            placeholder="Search runs by title"
            aria-label="Search runs"
            onChange={(event) => setSearchDraft(event.target.value)}
            onBlur={() => {
              if (searchDraft.trim() !== filters.search) {
                applyFilters(withFilterChange(filters, { search: searchDraft.trim() }));
              }
            }}
          />
        </form>

        <SectionTabs
          className="flex-none border-b-0"
          value={segmentFor(filters)}
          items={STATUS_SEGMENTS}
          onValueChange={(value) =>
            applyFilters(withFilterChange(filters, { status: value === "all" ? [] : [value] }))
          }
        />

        <Select
          value={filters.window}
          onValueChange={(value) =>
            applyFilters(withFilterChange(filters, { window: value as SessionTimeWindow }))
          }
        >
          <SelectTrigger className="w-40 flex-none" aria-label="Time window">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {TIME_WINDOWS.map((window) => (
              <SelectItem key={window} value={window}>
                {TIME_WINDOW_LABELS[window]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        {filtered && (
          <Button
            variant="ghost"
            size="sm"
            className="flex-none"
            onClick={() => {
              setSearchDraft("");
              applyFilters(EMPTY_SESSION_FILTERS);
            }}
          >
            Clear filters
          </Button>
        )}
      </PageControlStrip>

      <PageColumns>
        <PageMain>
          {sessionsLoading ? (
            <div className="space-y-2">
              {Array.from({ length: 6 }).map((_, index) => (
                <Skeleton key={index} className="h-16 w-full" />
              ))}
            </div>
          ) : sessionsError ? (
            <EmptyState
              icon={<Activity />}
              title="Could not load runs"
              description="The sessions list failed to load. Check your connection and try again."
              action={<Button onClick={() => void refetchSessions()}>Retry</Button>}
            />
          ) : sessions.length === 0 ? (
            <EmptyState
              icon={<MessageSquare />}
              title={filtered ? "No runs match these filters" : "No runs yet"}
              description={
                filtered
                  ? "Every filter narrows the same set. Clear one to widen it."
                  : "Runs appear here as soon as an agent is started — from a chat, a schedule, a channel or the API."
              }
              action={
                filtered ? (
                  <Button
                    variant="outline"
                    onClick={() => {
                      setSearchDraft("");
                      applyFilters(EMPTY_SESSION_FILTERS);
                    }}
                  >
                    Clear filters
                  </Button>
                ) : undefined
              }
            />
          ) : (
            <div className="flex flex-col gap-2">
              {sessions.map((session) => (
                <SessionRow
                  key={session.id}
                  session={session}
                  agentName={session.agent_id ? agentLabel(session.agent_id) : undefined}
                />
              ))}
            </div>
          )}
        </PageMain>

        <PageRail>
          <SessionFacetRail
            filters={filters}
            facets={facets}
            error={facetsError as Error | null}
            agentLabel={agentLabel}
            onToggleStatus={(value) => applyFilters(toggleFilterValue(filters, "status", value))}
            onToggleSource={(value) => applyFilters(toggleFilterValue(filters, "source", value))}
            onToggleAgent={(agentId) => applyFilters(toggleAgentFilter(filters, agentId))}
            views={views.views}
            viewsAreSaved={views.viewsAreSaved}
            canSaveView={filtered && !views.isSaving}
            onSelectView={handleSelectView}
            onSaveView={handleSaveView}
            onRemoveView={(id) => void views.removeView(id)}
          />
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {total === 0 ? 0 : offset + 1}-{Math.min(offset + SESSIONS_PAGE_SIZE, total)} of{" "}
          {total} {pluralize(total, "run")}
        </span>
        {totalPages > 1 && (
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={filters.page === 0}
              onClick={() => applyFilters({ ...filters, page: filters.page - 1 })}
            >
              <ChevronLeft className="size-4" />
              Previous
            </Button>
            <span>
              Page {filters.page + 1} of {totalPages}
            </span>
            <Button
              variant="outline"
              size="sm"
              disabled={filters.page >= totalPages - 1}
              onClick={() => applyFilters({ ...filters, page: filters.page + 1 })}
            >
              Next
              <ChevronRight className="size-4" />
            </Button>
          </div>
        )}
      </PageFooter>
    </PageContainer>
  );
}
