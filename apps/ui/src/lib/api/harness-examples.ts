// Harness Examples API — read-only examples adoptable as real Harnesses

import { api } from "./client";
import type { Harness, HarnessExample } from "./types";

export async function listHarnessExamples(): Promise<HarnessExample[]> {
  const response = await api.get<HarnessExample[]>("/v1/harness-examples");
  return response.data;
}

export async function importHarnessExample(name: string): Promise<Harness> {
  const response = await api.post<Harness>(
    `/v1/harnesses/import?from-example=${encodeURIComponent(name)}`,
  );
  return response.data;
}
