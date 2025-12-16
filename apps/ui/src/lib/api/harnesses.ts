// Harness API functions (M2)

import { api } from "./client";
import type {
  Harness,
  CreateHarnessRequest,
  UpdateHarnessRequest,
  ListResponse,
} from "./types";

export async function createHarness(
  request: CreateHarnessRequest
): Promise<Harness> {
  const response = await api.post<Harness>("/v1/harnesses", request);
  return response.data;
}

export async function listHarnesses(): Promise<Harness[]> {
  const response = await api.get<ListResponse<Harness>>("/v1/harnesses");
  return response.data.data;
}

export async function getHarness(harnessId: string): Promise<Harness> {
  const response = await api.get<Harness>(`/v1/harnesses/${harnessId}`);
  return response.data;
}

export async function getHarnessBySlug(slug: string): Promise<Harness> {
  const response = await api.get<Harness>(`/v1/harnesses/slug/${slug}`);
  return response.data;
}

export async function updateHarness(
  harnessId: string,
  request: UpdateHarnessRequest
): Promise<Harness> {
  const response = await api.patch<Harness>(
    `/v1/harnesses/${harnessId}`,
    request
  );
  return response.data;
}

export async function deleteHarness(harnessId: string): Promise<void> {
  await api.delete(`/v1/harnesses/${harnessId}`);
}
