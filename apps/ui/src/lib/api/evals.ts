// Eval API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import { createCrudApi } from "./crud";
import type {
  Eval,
  CreateEvalRequest,
  UpdateEvalRequest,
  EvalCase,
  CreateEvalCaseRequest,
  EvalRun,
  CreateEvalRunRequest,
  EvalRunShareLink,
  EvalRunShareStatus,
  PublicEvalRun,
} from "./types";

export const evalsCrudApi = createCrudApi<Eval, CreateEvalRequest, UpdateEvalRequest>("/v1/evals");

export const createEval = evalsCrudApi.create;
export const listEvals = evalsCrudApi.list;
export const getEval = evalsCrudApi.get;
export const updateEval = evalsCrudApi.update;
export const deleteEval = evalsCrudApi.delete;
export const destroyEval = evalsCrudApi.destroy;

function evalPath(evalId: string): string {
  return `/v1/evals/${encodeURIComponent(evalId)}`;
}

function evalRunPath(evalId: string, runId: string): string {
  return `${evalPath(evalId)}/runs/${encodeURIComponent(runId)}`;
}

// Eval cases (nested under eval)
export async function listEvalCases(evalId: string): Promise<EvalCase[]> {
  const response = await api.get<{ data: EvalCase[] }>(`${evalPath(evalId)}/cases`);
  return response.data.data;
}

export async function createEvalCase(
  evalId: string,
  request: CreateEvalCaseRequest,
): Promise<EvalCase> {
  const response = await api.post<EvalCase>(`${evalPath(evalId)}/cases`, request);
  return response.data;
}

export async function deleteEvalCase(evalId: string, caseId: string): Promise<void> {
  await api.delete(`${evalPath(evalId)}/cases/${encodeURIComponent(caseId)}`);
}

// Eval runs (nested under eval)
export async function listEvalRuns(evalId: string): Promise<EvalRun[]> {
  const response = await api.get<{ data: EvalRun[] }>(`${evalPath(evalId)}/runs`);
  return response.data.data;
}

export async function getEvalRun(evalId: string, runId: string): Promise<EvalRun> {
  const response = await api.get<EvalRun>(evalRunPath(evalId, runId));
  return response.data;
}

export async function createEvalRun(
  evalId: string,
  request: CreateEvalRunRequest,
): Promise<EvalRun> {
  const response = await api.post<EvalRun>(`${evalPath(evalId)}/runs`, request);
  return response.data;
}

export async function cancelEvalRun(evalId: string, runId: string): Promise<void> {
  await api.post(`${evalRunPath(evalId, runId)}/cancel`, {});
}

// Read-only share links for a run.
export async function getRunShareStatus(
  evalId: string,
  runId: string,
): Promise<EvalRunShareStatus> {
  const response = await api.get<EvalRunShareStatus>(`${evalRunPath(evalId, runId)}/share`);
  return response.data;
}

export async function createRunShare(evalId: string, runId: string): Promise<EvalRunShareLink> {
  const response = await api.post<EvalRunShareLink>(`${evalRunPath(evalId, runId)}/share`, {});
  return response.data;
}

export async function revokeRunShare(evalId: string, runId: string): Promise<void> {
  await api.delete(`${evalRunPath(evalId, runId)}/share`);
}

// Public (unauthenticated) read of a shared run. Uses a bare fetch — no auth
// cookies/headers needed — so it works for anonymous viewers.
export async function getPublicEvalRun(token: string): Promise<PublicEvalRun> {
  const res = await fetch(`/api/v1/public/eval-runs/${encodeURIComponent(token)}`, {
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    throw new Error(res.status === 404 ? "not_found" : `request failed (${res.status})`);
  }
  return (await res.json()) as PublicEvalRun;
}
