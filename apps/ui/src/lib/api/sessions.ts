// Session API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, throwApiError } from "./client";
import type {
  Session,
  SessionStats,
  CreateSessionRequest,
  ForkSessionRequest,
  UpdateSessionRequest,
  SessionContextReport,
  SessionResolvedModelResponse,
  SessionFacetsResponse,
  PaginatedResponse,
  PaginationParams,
} from "./types";

// Re-export message and event functions for convenience
export { createMessage, listMessages, sendUserMessage } from "./messages";
export { listEvents } from "./events";

// ============================================
// Session CRUD
// ============================================

/** Create a new organization session bound to an agent or directly to a harness. */
export async function createSession(request: CreateSessionRequest): Promise<Session> {
  const response = await api.post<Session>("/v1/sessions", request);
  return response.data;
}

/**
 * Filters `GET /v1/sessions` and `GET /v1/sessions/facets` share. Both
 * endpoints resolve them into the same predicate server-side, which is what
 * lets the facet counts annotate the very rows the list returns.
 */
export interface SessionListFilters {
  /** Filter by agent public id. */
  agentId?: string;
  /** Case-insensitive title substring match. */
  search?: string;
  /** Comma-separated sources (`chat`, `slack`, `schedule`, …). */
  source?: string;
  /** Comma-separated derived activities (`running`, `failed`, …). */
  status?: string;
  /** Restrict to sessions owned by the calling user. */
  mine?: boolean;
  /** Include archived sessions. Defaults to false server-side. */
  includeArchived?: boolean;
  /** Inclusive lower bound on creation time (RFC 3339). */
  createdAfter?: string;
  /** Exclusive upper bound on creation time (RFC 3339). */
  createdBefore?: string;
  /** `created_at` (default) or `last_activity`. */
  order?: "created_at" | "last_activity";
}

function sessionFilterParams(filters?: SessionListFilters): URLSearchParams {
  const searchParams = new URLSearchParams();
  if (filters?.agentId) searchParams.set("agent_id", filters.agentId);
  if (filters?.search) searchParams.set("search", filters.search);
  if (filters?.source) searchParams.set("source", filters.source);
  if (filters?.status) searchParams.set("status", filters.status);
  if (filters?.mine) searchParams.set("mine", "true");
  if (filters?.includeArchived) searchParams.set("include_archived", "true");
  if (filters?.createdAfter) searchParams.set("created_after", filters.createdAfter);
  if (filters?.createdBefore) searchParams.set("created_before", filters.createdBefore);
  if (filters?.order) searchParams.set("order", filters.order);
  return searchParams;
}

/**
 * List sessions for an organization.
 */
export async function listSessions(
  params?: PaginationParams & SessionListFilters,
): Promise<PaginatedResponse<Session>> {
  const searchParams = sessionFilterParams(params);
  if (params?.offset !== undefined) {
    searchParams.set("offset", String(params.offset));
  }
  if (params?.limit !== undefined) {
    searchParams.set("limit", String(params.limit));
  }
  const query = searchParams.toString();
  const url = `/v1/sessions${query ? `?${query}` : ""}`;
  const response = await api.get<PaginatedResponse<Session>>(url);
  return response.data;
}

/**
 * Facet-rail counts and masthead metrics over the same filter predicate as
 * {@link listSessions}. Counts are aggregated server-side — the page never
 * derives them by walking pages, so they stay correct beyond the first 20 rows.
 */
export async function getSessionFacets(
  filters?: SessionListFilters,
): Promise<SessionFacetsResponse> {
  const query = sessionFilterParams(filters).toString();
  const response = await api.get<SessionFacetsResponse>(
    `/v1/sessions/facets${query ? `?${query}` : ""}`,
  );
  return response.data;
}

/** Get session counts grouped by status */
export async function getSessionStats(): Promise<SessionStats> {
  const response = await api.get<SessionStats>("/v1/sessions/stats");
  return response.data;
}

export async function getSession(sessionId: string): Promise<Session> {
  const response = await api.get<Session>(`/v1/sessions/${sessionId}`);
  return response.data;
}

export async function getSessionResolvedModel(
  sessionId: string,
): Promise<SessionResolvedModelResponse> {
  const response = await api.get<SessionResolvedModelResponse>(
    `/v1/sessions/${sessionId}/resolved-model`,
  );
  return response.data;
}

export async function getSessionContextReport(sessionId: string): Promise<SessionContextReport> {
  const response = await api.get<SessionContextReport>(`/v1/sessions/${sessionId}/context-report`);
  return response.data;
}

export async function updateSession(
  sessionId: string,
  request: UpdateSessionRequest,
): Promise<Session> {
  const response = await api.patch<Session>(`/v1/sessions/${sessionId}`, request);
  return response.data;
}

export async function deleteSession(sessionId: string): Promise<void> {
  await api.delete(`/v1/sessions/${sessionId}`);
}

/**
 * Fork a session into an independent copy
 * (knowledge/runtime-resources/forking-sessions.md).
 *
 * The server seeds the fork with the parent's conversation events, workspace
 * files and durable session storage, so the fork can answer questions that
 * depend on the original transcript. Every request field is optional; omitted
 * fields inherit the parent's value.
 */
export async function forkSession(
  sessionId: string,
  request: ForkSessionRequest = {},
): Promise<Session> {
  const response = await api.post<Session>(`/v1/sessions/${sessionId}/fork`, request);
  return response.data;
}

/**
 * Cancel the currently running turn in a session.
 *
 * This will:
 * 1. Cancel the underlying workflow execution
 * 2. Emit a turn.cancelled event
 * 3. Insert an agent message indicating the turn was cancelled
 * 4. Set the session status back to idle
 *
 * @throws Error if no turn is currently running (409 Conflict)
 */
export async function cancelTurn(sessionId: string): Promise<void> {
  await api.post(`/v1/sessions/${sessionId}/cancel`);
}

// ============================================
// Session Pinning
// ============================================

/** Pin a session for the current user */
export async function pinSession(sessionId: string): Promise<void> {
  await api.put(`/v1/sessions/${sessionId}/pin`);
}

/** Unpin a session for the current user */
export async function unpinSession(sessionId: string): Promise<void> {
  await api.delete(`/v1/sessions/${sessionId}/pin`);
}

// ============================================
// Session Archiving
// ============================================

/** Archive a session so it drops out of default list results. */
export async function archiveSession(sessionId: string): Promise<void> {
  await api.put(`/v1/sessions/${sessionId}/archive`);
}

/** Restore an archived session to default list results. */
export async function unarchiveSession(sessionId: string): Promise<void> {
  await api.delete(`/v1/sessions/${sessionId}/archive`);
}

// ============================================
// Session Export
// ============================================

export type SessionExportFormat = "jsonl" | "atif";

export interface SessionExportResult {
  /**
   * Number of image parts the server omitted from an ATIF document
   * (`X-Atif-Images-Omitted` header). 0 when the header is absent (JSONL
   * exports, or servers without ATIF support).
   */
  imagesOmitted: number;
}

/**
 * Export session messages and trigger a browser download.
 * `jsonl` (default) downloads `{sessionId}.jsonl`; `atif` downloads the
 * ATIF-v1.7 trajectory document as `{sessionId}.atif.json`.
 *
 * Throws `ApiError` on failure — notably status 413 when the ATIF document
 * exceeds the server's size cap.
 */
export async function exportSession(
  sessionId: string,
  format: SessionExportFormat = "jsonl",
): Promise<SessionExportResult> {
  const query = format === "atif" ? "?format=atif" : "";
  const response = await fetch(`/api/v1/sessions/${sessionId}/export${query}`, {
    credentials: "include",
  });
  if (!response.ok) {
    await throwApiError(response);
  }
  const omitted = parseImagesOmitted(response.headers.get("X-Atif-Images-Omitted"));
  const blob = await response.blob();
  triggerBlobDownload(format === "atif" ? `${sessionId}.atif.json` : `${sessionId}.jsonl`, blob);
  return { imagesOmitted: omitted };
}

// ============================================
// Segmented ATIF Export
// ============================================

/**
 * Safety cap on the segmented walk. A server that never stops emitting
 * `continued_trajectory_ref` (bug, or hostile) must not loop forever or fill
 * the disk. 10k segments is far beyond any legitimate session.
 */
export const MAX_ATIF_SEGMENTS = 10_000;

export interface SegmentedExportResult {
  /** Number of segment files saved to disk. */
  partsSaved: number;
  /** Total images omitted across all segments (summed `X-Atif-Images-Omitted`). */
  imagesOmitted: number;
}

/**
 * Thrown when the segmented walk aborts. Carries how many parts were saved
 * before the failure so callers can say "stopped after N parts".
 */
export class SegmentedExportError extends Error {
  constructor(
    message: string,
    public partsSaved: number,
    /** `http`: a segment request failed. `limit`: hit `MAX_ATIF_SEGMENTS`. */
    public reason: "http" | "limit",
    public apiError?: unknown,
  ) {
    super(message);
    this.name = "SegmentedExportError";
  }
}

function parseImagesOmitted(header: string | null): number {
  const n = header ? Number.parseInt(header, 10) : 0;
  return Number.isFinite(n) && n > 0 ? n : 0;
}

function triggerBlobDownload(filename: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

/**
 * Extract the opaque `cursor` from a server-provided next-segment reference.
 *
 * Security: the server returns a full-path `continued_trajectory_ref` URL, but
 * we deliberately trust only its `cursor` query value and always re-fetch our
 * own same-origin `/api` export endpoint. This prevents following the returned
 * ref to an arbitrary path/origin. The cursor stays opaque server state.
 */
function cursorFromRef(ref: string | null | undefined): string | null {
  if (!ref) return null;
  try {
    // Parse against a dummy base so relative refs still expose their query.
    return new URL(ref, "http://ref.invalid").searchParams.get("cursor");
  } catch {
    return null;
  }
}

/**
 * The `X-Atif-Next-Cursor` header may carry either a full ref URL or the raw
 * cursor value; accept both, still opaquely.
 */
function cursorFromHeader(value: string | null): string | null {
  if (!value) return null;
  const fromUrl = cursorFromRef(value);
  if (fromUrl) return fromUrl;
  // A bare token (no query/path syntax) is the raw cursor.
  return value.includes("=") || value.includes("/") ? null : value.trim() || null;
}

function nextSegmentUrl(sessionId: string, cursor: string | null): string {
  const params = new URLSearchParams({ format: "atif", segmented: "true" });
  if (cursor) params.set("cursor", cursor);
  return `/api/v1/sessions/${sessionId}/export?${params.toString()}`;
}

/**
 * Walk the segmented ATIF export, saving each segment as a separate file
 * (`{sessionId}.atif.part{n}.json`, 1-based). Follows `continued_trajectory_ref`
 * (via its opaque cursor) until a segment omits it. Recoverable path for
 * sessions too large for a single ATIF document (HTTP 413).
 */
export async function exportSessionSegmented(sessionId: string): Promise<SegmentedExportResult> {
  let cursor: string | null = null;
  let partsSaved = 0;
  let imagesOmitted = 0;

  for (;;) {
    let response: Response;
    try {
      response = await fetch(nextSegmentUrl(sessionId, cursor), { credentials: "include" });
    } catch (err) {
      throw new SegmentedExportError(
        `Segmented ATIF export failed after ${partsSaved} part(s)`,
        partsSaved,
        "http",
        err,
      );
    }

    if (!response.ok) {
      let apiError: unknown;
      try {
        await throwApiError(response);
      } catch (err) {
        apiError = err;
      }
      throw new SegmentedExportError(
        `Segmented ATIF export failed after ${partsSaved} part(s)`,
        partsSaved,
        "http",
        apiError,
      );
    }

    imagesOmitted += parseImagesOmitted(response.headers.get("X-Atif-Images-Omitted"));
    const text = await response.text();
    triggerBlobDownload(
      `${sessionId}.atif.part${partsSaved + 1}.json`,
      new Blob([text], { type: "application/json" }),
    );
    partsSaved += 1;

    // Primary continuation signal is the body's root `continued_trajectory_ref`;
    // the `X-Atif-Next-Cursor` header is a fallback. The final segment omits both.
    let bodyRef: string | null = null;
    try {
      const doc = JSON.parse(text) as { continued_trajectory_ref?: unknown };
      if (typeof doc.continued_trajectory_ref === "string") {
        bodyRef = doc.continued_trajectory_ref;
      }
    } catch {
      // Non-JSON body: rely on the header fallback below.
    }

    const nextCursor =
      cursorFromRef(bodyRef) ?? cursorFromHeader(response.headers.get("X-Atif-Next-Cursor"));
    if (!nextCursor) {
      return { partsSaved, imagesOmitted };
    }
    if (partsSaved >= MAX_ATIF_SEGMENTS) {
      throw new SegmentedExportError(
        `Segmented ATIF export exceeded ${MAX_ATIF_SEGMENTS} parts`,
        partsSaved,
        "limit",
      );
    }
    cursor = nextCursor;
  }
}

// ============================================
// Client-Side Tool Results
// ============================================

/**
 * Submit tool results for client-side tool calls.
 * Resumes a session that is paused in `waiting_for_tool_results` status.
 */
export async function submitToolResults(
  sessionId: string,
  toolResults: Array<{ tool_call_id: string; result?: unknown; error?: string }>,
): Promise<{ status: string; tool_results_count: number }> {
  const response = await api.post<{ status: string; tool_results_count: number }>(
    `/v1/sessions/${sessionId}/tool-results`,
    { tool_results: toolResults },
  );
  return response.data;
}

// ============================================
// URL Mode Elicitation Consent
// ============================================

/**
 * Answer a URL an MCP server asked the user to open.
 *
 * Only the decision travels: the server, tool and domain it applies to come
 * from the `confirm_url_elicitation` call the backend itself emitted, so the
 * browser cannot record consent for something the user was never shown.
 */
export async function submitElicitationConsent(
  sessionId: string,
  toolCallId: string,
  action: "accept" | "decline",
): Promise<{ host: string; status: string }> {
  const response = await api.post<{ host: string; status: string }>(
    `/v1/sessions/${sessionId}/mcp-elicitation-consent`,
    { tool_call_id: toolCallId, action },
  );
  return response.data;
}
