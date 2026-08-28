// Event API functions
// Events are SSE notifications for real-time updates
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { getApiBaseUrl, throwApiError } from "./client";
import type { Event, ListResponse } from "./types";

// Default event types to exclude in UI contexts (streaming delta events are noise)
export const DEFAULT_EXCLUDED_EVENTS = ["output.message.delta", "reason.thinking.delta"];

// Options for listing events with pagination support
export interface ListEventsOptions {
  exclude?: string[];
  /** Max events to return (backward pagination). Returns last N events. */
  limit?: number;
  /** Cursor: only return events with sequence < this value. Used with limit. */
  before_sequence?: number;
}

// Response from paginated event listing
export interface ListEventsResult {
  events: Event[];
  /** Total non-delta event count (from X-Total-Count header, only when limit is used) */
  totalNonDeltaCount?: number;
}

// List events for a session (polling alternative to SSE)
// Supports backward pagination via limit + before_sequence params.
// When limit is set, response includes X-Total-Count header with non-delta event count.
export async function listEvents(
  sessionId: string,
  excludeOrOptions?: string[] | ListEventsOptions,
): Promise<Event[]> {
  const result = await listEventsPaginated(sessionId, excludeOrOptions);
  return result.events;
}

// List events with pagination metadata (X-Total-Count header).
// Use this when you need the total count for scroll estimation.
// Uses fetch directly (instead of api.get) to access response headers.
export async function listEventsPaginated(
  sessionId: string,
  excludeOrOptions?: string[] | ListEventsOptions,
): Promise<ListEventsResult> {
  const options: ListEventsOptions = Array.isArray(excludeOrOptions)
    ? { exclude: excludeOrOptions }
    : (excludeOrOptions ?? {});

  const params = new URLSearchParams();
  if (options.exclude) {
    for (const type of options.exclude) {
      params.append("exclude", type);
    }
  }
  if (options.limit !== undefined) {
    params.set("limit", String(options.limit));
  }
  if (options.before_sequence !== undefined) {
    params.set("before_sequence", String(options.before_sequence));
  }
  const queryString = params.toString();
  const url = `${getApiBaseUrl()}/v1/sessions/${sessionId}/events${queryString ? `?${queryString}` : ""}`;

  const response = await fetch(url, {
    method: "GET",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
  });

  if (!response.ok) {
    await throwApiError(response);
  }

  const body: ListResponse<Event> = await response.json();
  const totalHeader = response.headers.get("x-total-count");
  const totalNonDeltaCount = totalHeader ? Number(totalHeader) : undefined;

  return {
    events: body.data,
    totalNonDeltaCount,
  };
}

/**
 * Cursor the SSE stream resumes from.
 *
 * `sinceId` is the normal case: everything after that event is replayed, then the
 * stream goes live. A client that holds no events at all cannot express that with
 * an id, so it sends `afterSequence: 0` — replay the session from its first event.
 * Without a cursor the server starts live, and anything written between the
 * client's REST snapshot and the subscription is dropped for good.
 */
export interface SseCursor {
  sinceId?: string;
  afterSequence?: number;
}

// Get SSE URL for real-time event streaming
// Takes the cursor to resume from (see SseCursor) and an optional exclude list
// filtering out event types (e.g. delta events)
// Note: Org is sent via everruns_org cookie (EventSource sends cookies automatically)
export function getSseUrl(sessionId: string, cursor?: SseCursor, exclude?: string[]): string {
  const baseUrl = getApiBaseUrl();
  const params = new URLSearchParams();
  const { sinceId, afterSequence } = cursor ?? {};
  if (sinceId) {
    params.set("since_id", sinceId);
  } else if (afterSequence !== undefined) {
    // Mutually exclusive with since_id on the server.
    params.set("after_sequence", String(afterSequence));
  }
  if (exclude) {
    for (const type of exclude) {
      params.append("exclude", type);
    }
  }
  const queryString = params.toString();
  return `${baseUrl}/v1/sessions/${sessionId}/sse${queryString ? `?${queryString}` : ""}`;
}
