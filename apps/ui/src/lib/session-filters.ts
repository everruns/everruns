// Sessions list filter state (EVE-853).
//
// The filter set lives in the URL, not in component state: a saved view is then
// just a link, and the page, the facet counts and a shared URL can never
// describe different populations. This module is the single translator between
// the URL, the component props, and the query string the API expects — so the
// list request and the facets request are always built from one value.

import type { SessionActivity, SessionSource } from "@/lib/api/types";

/** Page size for the operational list. */
export const SESSIONS_PAGE_SIZE = 20;

/**
 * Time windows offered by the filter strip. Bounds are applied to session
 * creation time, matching the `created_after` predicate the API filters on.
 */
export const TIME_WINDOWS = ["all", "24h", "7d", "30d"] as const;
export type SessionTimeWindow = (typeof TIME_WINDOWS)[number];

export const TIME_WINDOW_LABELS: Record<SessionTimeWindow, string> = {
  all: "Any time",
  "24h": "Last 24h",
  "7d": "Last 7 days",
  "30d": "Last 30 days",
};

const WINDOW_DURATION_MS: Record<Exclude<SessionTimeWindow, "all">, number> = {
  "24h": 24 * 60 * 60 * 1000,
  "7d": 7 * 24 * 60 * 60 * 1000,
  "30d": 30 * 24 * 60 * 60 * 1000,
};

export const ACTIVITY_LABELS: Record<SessionActivity, string> = {
  running: "Running",
  failed: "Failed",
  completed: "Completed",
  paused: "Paused",
  idle: "Idle",
};

export const SOURCE_LABELS: Record<SessionSource, string> = {
  chat: "Chat",
  api: "API",
  slack: "Slack",
  ag_ui: "AG-UI",
  fcp: "FCP",
  schedule: "Schedule",
  webhook: "Webhook",
  a2a: "A2A",
  eval: "Eval",
  subagent: "Subagent",
  unknown: "Unknown",
};

/** Human label for a source value, tolerating values a newer server may add. */
export function sourceLabel(value: string): string {
  return SOURCE_LABELS[value as SessionSource] ?? value;
}

/** Human label for an activity value, tolerating values a newer server may add. */
export function activityLabel(value: string): string {
  return ACTIVITY_LABELS[value as SessionActivity] ?? value;
}

/**
 * The complete filter state of the sessions page. Everything here round-trips
 * through the URL; nothing about what is on screen lives only in memory.
 */
export interface SessionFilters {
  /** Case-insensitive title search. */
  search: string;
  /** Selected derived activities. Empty means "every activity". */
  status: string[];
  /** Selected sources. Empty means "every source". */
  source: string[];
  /**
   * Selected agent public id, or null for every agent. Single-valued because
   * `GET /v1/sessions` takes one `agent_id`; the rail must not offer a
   * selection the server cannot honour.
   */
  agent: string | null;
  /** Creation-time window. */
  window: SessionTimeWindow;
  /** Restrict to sessions owned by the signed-in user. */
  mine: boolean;
  /** Widen the list to archived sessions. Off by default — archiving is the
   *  user asking for a session to stop showing up. */
  includeArchived: boolean;
  /** Zero-based page index. */
  page: number;
}

export const EMPTY_SESSION_FILTERS: SessionFilters = {
  search: "",
  status: [],
  source: [],
  agent: null,
  window: "all",
  mine: false,
  includeArchived: false,
  page: 0,
};

function parseList(raw: string | null): string[] {
  if (!raw) return [];
  return [
    ...new Set(
      raw
        .split(",")
        .map((part) => part.trim())
        .filter(Boolean),
    ),
  ];
}

function parseWindow(raw: string | null): SessionTimeWindow {
  return TIME_WINDOWS.includes(raw as SessionTimeWindow) ? (raw as SessionTimeWindow) : "all";
}

/** Read the filter state out of a URL query string. Unknown values fall back. */
export function parseSessionFilters(params: URLSearchParams): SessionFilters {
  const page = Number.parseInt(params.get("page") ?? "", 10);
  return {
    search: params.get("q") ?? "",
    status: parseList(params.get("status")),
    source: parseList(params.get("source")),
    agent: params.get("agent") || null,
    window: parseWindow(params.get("window")),
    mine: params.get("mine") === "true",
    includeArchived: params.get("include_archived") === "true",
    page: Number.isFinite(page) && page > 0 ? page : 0,
  };
}

/**
 * Serialize filters back to a query string. Defaults are omitted so a pristine
 * list has a clean URL and two equivalent filter sets share one cache key.
 */
export function serializeSessionFilters(filters: SessionFilters): string {
  const params = new URLSearchParams();
  if (filters.search) params.set("q", filters.search);
  if (filters.status.length) params.set("status", filters.status.join(","));
  if (filters.source.length) params.set("source", filters.source.join(","));
  if (filters.agent) params.set("agent", filters.agent);
  if (filters.window !== "all") params.set("window", filters.window);
  if (filters.mine) params.set("mine", "true");
  if (filters.includeArchived) params.set("include_archived", "true");
  if (filters.page > 0) params.set("page", String(filters.page));
  return params.toString();
}

/** Whether any filter narrows the list (page position aside). */
export function hasActiveFilters(filters: SessionFilters): boolean {
  return (
    filters.search !== "" ||
    filters.status.length > 0 ||
    filters.source.length > 0 ||
    filters.agent !== null ||
    filters.window !== "all" ||
    filters.mine ||
    filters.includeArchived
  );
}

/**
 * Toggle one value of a multi-select facet, resetting to the first page.
 * Changing what is counted must never leave the reader on a page that the new
 * result set no longer has.
 */
export function toggleFilterValue(
  filters: SessionFilters,
  dimension: "status" | "source",
  value: string,
): SessionFilters {
  const current = filters[dimension];
  const next = current.includes(value)
    ? current.filter((entry) => entry !== value)
    : [...current, value];
  return { ...filters, [dimension]: next, page: 0 };
}

/** Select an agent, or clear the selection by toggling the active one. */
export function toggleAgentFilter(filters: SessionFilters, agentId: string): SessionFilters {
  return { ...filters, agent: filters.agent === agentId ? null : agentId, page: 0 };
}

/** Apply a partial change, resetting pagination unless the page itself moved. */
export function withFilterChange(
  filters: SessionFilters,
  change: Partial<SessionFilters>,
): SessionFilters {
  const next = { ...filters, ...change };
  return "page" in change ? next : { ...next, page: 0 };
}

/**
 * Lower bound on `created_at` for a window, as an RFC 3339 string.
 *
 * Floored to the start of the minute so the value is stable across renders —
 * an unquantized `Date.now()` would mint a new query key (and a new request)
 * on every keystroke.
 */
export function windowStart(
  window: SessionTimeWindow,
  now: number = Date.now(),
): string | undefined {
  if (window === "all") return undefined;
  const anchor = Math.floor(now / 60_000) * 60_000;
  return new Date(anchor - WINDOW_DURATION_MS[window]).toISOString();
}

/** Query parameters shared by the list request and the facets request. */
export interface SessionQueryParams {
  search?: string;
  status?: string;
  source?: string;
  agentId?: string;
  mine?: boolean;
  includeArchived?: boolean;
  createdAfter?: string;
}

/** Translate filters into the parameters both API calls take. */
export function toQueryParams(
  filters: SessionFilters,
  now: number = Date.now(),
): SessionQueryParams {
  return {
    search: filters.search || undefined,
    status: filters.status.length ? filters.status.join(",") : undefined,
    source: filters.source.length ? filters.source.join(",") : undefined,
    agentId: filters.agent ?? undefined,
    mine: filters.mine || undefined,
    includeArchived: filters.includeArchived || undefined,
    createdAfter: windowStart(filters.window, now),
  };
}
