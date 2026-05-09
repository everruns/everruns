import type { AgentCapabilityConfig } from "@/lib/api/types";

export function capabilityConfigRecord(config: unknown): Record<string, unknown> {
  if (!config || typeof config !== "object" || Array.isArray(config)) {
    return {};
  }
  return config as Record<string, unknown>;
}

export function normalizeCapabilityConfigs(
  capabilities: AgentCapabilityConfig[] | null | undefined,
): AgentCapabilityConfig[] {
  return (capabilities ?? []).map((capability) => ({
    ...capability,
    config: capabilityConfigRecord(capability.config),
  }));
}
