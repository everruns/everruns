// Feature flags API client
import { api } from "./client";
import type { FeatureFlags } from "./types";

export async function getFeatureFlags(): Promise<FeatureFlags> {
  const { data } = await api.get<FeatureFlags>("/v1/feature-flags");
  return data;
}
