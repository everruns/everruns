// Observer API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import { createCrudApi } from "./crud";
import type {
  Observer,
  CreateObserverRequest,
  UpdateObserverRequest,
  QualitySummary,
  TraceScore,
} from "./types";

export const observersCrudApi = createCrudApi<
  Observer,
  CreateObserverRequest,
  UpdateObserverRequest
>("/v1/observers");

export const createObserver = observersCrudApi.create;
export const listObservers = observersCrudApi.list;
export const getObserver = observersCrudApi.get;
export const updateObserver = observersCrudApi.update;
export const deleteObserver = observersCrudApi.delete;
export const destroyObserver = observersCrudApi.destroy;

export interface ListObserverScoresParams {
  session_id?: string;
  /** Only unreviewed completed scores, failures first. */
  review_queue?: boolean;
  limit?: number;
  offset?: number;
}

// Trace scores (nested under observer)
export async function listObserverScores(
  observerId: string,
  params: ListObserverScoresParams = {},
): Promise<TraceScore[]> {
  const search = new URLSearchParams();
  if (params.session_id) search.set("session_id", params.session_id);
  if (params.review_queue) search.set("review_queue", "true");
  if (params.limit !== undefined) search.set("limit", String(params.limit));
  if (params.offset !== undefined) search.set("offset", String(params.offset));
  const query = search.toString();
  const path = query
    ? `/v1/observers/${observerId}/scores?${query}`
    : `/v1/observers/${observerId}/scores`;
  const response = await api.get<{ data: TraceScore[] }>(path);
  return response.data.data;
}

export interface ObserverSummaryParams {
  days?: number;
  agent_id?: string;
  observer_id?: string;
}

/** Live quality: how production traffic scored, by agent, observer and day. */
export async function getObserverSummary(
  params: ObserverSummaryParams = {},
): Promise<QualitySummary> {
  const search = new URLSearchParams();
  if (params.days !== undefined) search.set("days", String(params.days));
  if (params.agent_id) search.set("agent_id", params.agent_id);
  if (params.observer_id) search.set("observer_id", params.observer_id);
  const query = search.toString();
  const response = await api.get<QualitySummary>(
    query ? `/v1/observers/summary?${query}` : "/v1/observers/summary",
  );
  return response.data;
}
