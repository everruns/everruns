// Harness API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import { createCrudApi } from "./crud";
import type {
  AgentPreviewResponse,
  CreateHarnessRequest,
  Harness,
  PreviewHarnessRequest,
  UpdateHarnessRequest,
} from "./types";

export const harnessesCrudApi = createCrudApi<Harness, CreateHarnessRequest, UpdateHarnessRequest>(
  "/v1/harnesses",
);

export const createHarness = harnessesCrudApi.create;
export const listHarnesses = harnessesCrudApi.list;
export const getHarness = harnessesCrudApi.get;
export const updateHarness = harnessesCrudApi.update;
export const deleteHarness = harnessesCrudApi.delete;
export const destroyHarness = harnessesCrudApi.destroy;

export async function copyHarness(harnessId: string): Promise<Harness> {
  const response = await api.post<Harness>(`/v1/harnesses/${harnessId}/copy`, {});
  return response.data;
}

export async function previewHarness(
  request: PreviewHarnessRequest,
): Promise<AgentPreviewResponse> {
  const response = await api.post<AgentPreviewResponse>("/v1/harnesses/preview", request);
  return response.data;
}
