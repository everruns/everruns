// Helpers for linking from the chat UI to a provider's observability dashboard
// ("trace"/"logs"). The mechanism is provider-agnostic: each provider carries a
// resolved `ProviderTraceConfig` (driver defaults overlaid with org overrides),
// and the chat UI builds deep links from URL templates with `{response_id}`,
// `{session_id}`, `{turn_id}` and `{model}` placeholders.

import { isRecord } from "@/lib/api/types";
import type { DriverId, Provider, ProviderTraceConfig } from "@/lib/api/types";

export interface TraceUrlParams {
  responseId?: string;
  sessionId?: string;
  turnId?: string;
  model?: string;
}

/**
 * Substitute placeholders in a trace URL template. Returns `null` when the
 * template references a placeholder we have no value for, so the caller can hide
 * the link rather than render a broken URL.
 */
export function buildTraceUrl(template: string, params: TraceUrlParams): string | null {
  const replacements: Record<string, string | undefined> = {
    response_id: params.responseId,
    session_id: params.sessionId,
    turn_id: params.turnId,
    model: params.model,
  };

  let missing = false;
  const url = template.replace(/\{(\w+)\}/g, (match, key: string) => {
    const value = replacements[key];
    if (value == null || value === "") {
      missing = true;
      return match;
    }
    return encodeURIComponent(value);
  });

  return missing ? null : url;
}

/**
 * Build a `driverId -> ProviderTraceConfig` map from the providers list,
 * including only providers that have trace links enabled. When several providers
 * share a driver, the first enabled one wins (the common case is a single
 * provider per driver).
 */
export function buildTraceConfigByDriver(
  providers: Provider[] | undefined,
): Map<DriverId, ProviderTraceConfig> {
  const map = new Map<DriverId, ProviderTraceConfig>();
  for (const provider of providers ?? []) {
    const trace = provider.trace;
    if (trace?.enabled && !map.has(provider.provider_type)) {
      map.set(provider.provider_type, trace);
    }
  }
  return map;
}

/**
 * Resolve a deep link to a single generation's trace from an assistant message's
 * metadata (the `provider` driver id and `response_id` stamped by the reason
 * atom). Returns `null` when no enabled config or template applies.
 */
export function resolveGenerationTraceUrl(
  metadata: unknown,
  traceByDriver: Map<DriverId, ProviderTraceConfig>,
): string | null {
  if (!isRecord(metadata)) return null;
  const provider =
    typeof metadata.provider === "string" ? (metadata.provider as DriverId) : undefined;
  const responseId = typeof metadata.response_id === "string" ? metadata.response_id : undefined;
  if (!provider) return null;
  const config = traceByDriver.get(provider);
  if (!config?.enabled || !config.generation_url_template) return null;
  return buildTraceUrl(config.generation_url_template, { responseId });
}

/**
 * Resolve a link to the session's grouped trace for the given driver.
 */
export function resolveSessionTraceUrl(
  providerType: DriverId | undefined,
  sessionId: string,
  traceByDriver: Map<DriverId, ProviderTraceConfig>,
): string | null {
  if (!providerType) return null;
  const config = traceByDriver.get(providerType);
  if (!config?.enabled || !config.session_url_template) return null;
  return buildTraceUrl(config.session_url_template, { sessionId });
}
