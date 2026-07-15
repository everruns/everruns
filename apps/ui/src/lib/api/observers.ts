// Observer API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import { createCrudApi } from "./crud";
import type { Observer, CreateObserverRequest, UpdateObserverRequest, TraceScore } from "./types";

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
  if (params.limit !== undefined) search.set("limit", String(params.limit));
  if (params.offset !== undefined) search.set("offset", String(params.offset));
  const query = search.toString();
  const path = query
    ? `/v1/observers/${observerId}/scores?${query}`
    : `/v1/observers/${observerId}/scores`;
  const response = await api.get<{ data: TraceScore[] }>(path);
  return response.data.data;
}
