// Session trace API: the bounded reads behind the session Trace tab.
// See knowledge/ui/session-trace.md.

import { api } from "./client";
import type {
  TraceEventsPage,
  TraceOverview,
  TraceRequestPage,
  TraceStepDetail,
  TraceStepsPage,
  TraceTurnsPage,
} from "./types";

function query(params: Record<string, string | number | boolean | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) search.set(key, String(value));
  }
  const text = search.toString();
  return text ? `?${text}` : "";
}

const base = (sessionId: string) => `/v1/sessions/${sessionId}/trace`;

/** Totals, the minimap and the turns that failed. */
export async function getSessionTrace(sessionId: string, buckets?: number): Promise<TraceOverview> {
  const response = await api.get<TraceOverview>(`${base(sessionId)}${query({ buckets })}`);
  return response.data;
}

export interface TraceTurnsCursor {
  before?: number;
  after?: number;
  around?: number;
  sequence?: number;
  limit?: number;
}

/** A page of turns with their steps; without a cursor, the last turns. */
export async function listSessionTraceTurns(
  sessionId: string,
  cursor: TraceTurnsCursor = {},
): Promise<TraceTurnsPage> {
  const response = await api.get<TraceTurnsPage>(`${base(sessionId)}/turns${query({ ...cursor })}`);
  return response.data;
}

export interface TraceStepsRange {
  from_step?: number;
  to_step?: number;
  errors_only?: boolean;
  limit?: number;
}

/** Plain steps of one turn, for gaps and batch members. */
export async function listSessionTraceSteps(
  sessionId: string,
  turn: number,
  range: TraceStepsRange = {},
): Promise<TraceStepsPage> {
  const response = await api.get<TraceStepsPage>(
    `${base(sessionId)}/turns/${turn}/steps${query({ ...range })}`,
  );
  return response.data;
}

/** Everything about one step: input, output, request summary, raw events. */
export async function getSessionTraceStep(
  sessionId: string,
  turn: number,
  step: number,
  full?: boolean,
): Promise<TraceStepDetail> {
  const response = await api.get<TraceStepDetail>(
    `${base(sessionId)}/turns/${turn}/steps/${step}${query({ full: full || undefined })}`,
  );
  return response.data;
}

/** The messages a model call was sent, with full content. */
export async function listSessionTraceRequest(
  sessionId: string,
  turn: number,
  step: number,
  options: { role?: string; offset?: number; limit?: number } = {},
): Promise<TraceRequestPage> {
  const response = await api.get<TraceRequestPage>(
    `${base(sessionId)}/turns/${turn}/steps/${step}/request${query({ ...options })}`,
  );
  return response.data;
}

/** Raw events of one turn, without deltas. */
export async function listSessionTraceTurnEvents(
  sessionId: string,
  turn: number,
  options: { after_sequence?: number; limit?: number } = {},
): Promise<TraceEventsPage> {
  const response = await api.get<TraceEventsPage>(
    `${base(sessionId)}/turns/${turn}/events${query({ ...options })}`,
  );
  return response.data;
}
