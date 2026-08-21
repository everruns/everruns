// CSV export of the filtered sessions list (EVE-853).
//
// The masthead Export exports what the filters describe, not what happens to be
// on the current page — otherwise the button would quietly mean "export 20
// rows". Rows are pulled from the same endpoint the list renders, so an export
// and the screen can never disagree.

import { listSessions, type SessionListFilters } from "@/lib/api/sessions";
import type { Session } from "@/lib/api/types";

/** Server maximum for `limit` on `GET /v1/sessions`. */
const EXPORT_PAGE_SIZE = 100;

/**
 * Ceiling on exported rows. An unbounded export of an org's whole history is a
 * request nobody means to make from a list page; the caller is told when the
 * result was cut so the number is never silently wrong.
 */
export const EXPORT_ROW_LIMIT = 2_000;

export interface SessionsCsvResult {
  rowsExported: number;
  /** True when the filtered set was larger than {@link EXPORT_ROW_LIMIT}. */
  truncated: boolean;
}

function csvCell(value: string | number | null | undefined): string {
  if (value === null || value === undefined) return "";
  const rawText = String(value);
  // THREAT[TM-API-019]: Neutralize spreadsheet formulas before RFC 4180 escaping.
  const text = /^[=+@\t\r\n-]/.test(rawText) ? `'${rawText}` : rawText;
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

const HEADER = [
  "id",
  "source",
  "status",
  "agent",
  "title",
  "created_at",
  "started_at",
  "finished_at",
  "duration_ms",
  "input_tokens",
  "output_tokens",
  "cost_usd",
];

function durationMs(session: Session): number | undefined {
  if (!session.started_at || !session.finished_at) return undefined;
  const ms = new Date(session.finished_at).getTime() - new Date(session.started_at).getTime();
  return Number.isFinite(ms) && ms >= 0 ? ms : undefined;
}

function toRow(session: Session, agentName: (agentId: string) => string): string {
  return [
    session.id,
    session.source,
    session.activity,
    session.agent_id ? agentName(session.agent_id) : "",
    session.title ?? "",
    session.created_at,
    session.started_at ?? "",
    session.finished_at ?? "",
    durationMs(session) ?? "",
    session.usage?.input_tokens ?? "",
    session.usage?.output_tokens ?? "",
    session.usage?.effective_cost_usd ?? session.usage?.actual_cost_usd ?? "",
  ]
    .map(csvCell)
    .join(",");
}

/** Fetch every session matching `filters`, up to {@link EXPORT_ROW_LIMIT}. */
async function fetchFilteredSessions(
  filters: SessionListFilters,
): Promise<{ sessions: Session[]; truncated: boolean }> {
  const sessions: Session[] = [];
  let offset = 0;
  let total = Infinity;

  while (sessions.length < Math.min(total, EXPORT_ROW_LIMIT)) {
    const page = await listSessions({ ...filters, offset, limit: EXPORT_PAGE_SIZE });
    total = page.total;
    if (page.data.length === 0) break;
    sessions.push(...page.data);
    offset += page.data.length;
  }

  return {
    sessions: sessions.slice(0, EXPORT_ROW_LIMIT),
    truncated: total > EXPORT_ROW_LIMIT,
  };
}

/**
 * Build a CSV of the filtered sessions and trigger a browser download.
 * Returns how many rows were written so the caller can report it.
 */
export async function downloadSessionsCsv(
  filters: SessionListFilters,
  agentName: (agentId: string) => string,
): Promise<SessionsCsvResult> {
  const { sessions, truncated } = await fetchFilteredSessions(filters);
  const csv = [HEADER.join(","), ...sessions.map((session) => toRow(session, agentName))].join(
    "\n",
  );

  const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "sessions.csv";
  anchor.click();
  URL.revokeObjectURL(url);

  return { rowsExported: sessions.length, truncated };
}
