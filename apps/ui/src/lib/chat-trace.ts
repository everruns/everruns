// Helpers for linking from the chat UI to a provider's observability dashboard
// ("trace"/"logs"). The mechanism is provider-agnostic: each provider carries a
// resolved `ProviderTraceConfig` (driver defaults overlaid with org overrides),
// and the chat UI builds deep links from URL templates with `{response_id}`,
// `{session_id}`, `{turn_id}` and `{model}` placeholders.
//
// Trace config is keyed by driver id. The runtime's resolved model (and thus the
// assistant-message metadata) carries the driver id, not the concrete provider
// instance id, so when an org runs multiple providers on the same driver the
// first trace-enabled provider for that driver wins. Threading the provider
// instance id through the resolver and worker boundary is a separate change.

import { isRecord } from "@/lib/api/types";
import type { DriverId, Provider, ProviderTraceConfig } from "@/lib/api/types";

export interface TraceUrlParams {
  responseId?: string;
  sessionId?: string;
  turnId?: string;
  model?: string;
}

/**
 * Substitute placeholders in a trace URL template. Returns `null` when:
 * - the template references a placeholder we have no value for (so the caller
 *   hides the link rather than rendering a broken URL), or
 * - the result is not a valid `http(s)` URL. Templates come from org settings
 *   and are rendered as clickable `href`s, so rejecting `javascript:`/`data:`
 *   (and other) schemes prevents an XSS/phishing vector.
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

  if (missing) return null;

  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
  } catch {
    return null;
  }

  return url;
}

/**
 * Build a `driverId -> ProviderTraceConfig` map from the providers list,
 * including only providers that have trace links enabled. When several providers
 * share a driver, the first trace-enabled one wins (see module note).
 */
export function buildTraceConfigByDriver(
  providers: Provider[] | undefined,
): Map<DriverId, ProviderTraceConfig> {
  const map = new Map<DriverId, ProviderTraceConfig>();
  for (const provider of providers ?? []) {
    if (provider.trace?.enabled && !map.has(provider.provider_type)) {
      map.set(provider.provider_type, provider.trace);
    }
  }
  return map;
}

/**
 * Resolve a deep link to a single generation's trace from an assistant message's
 * metadata (the `provider` driver id and `response_id` stamped by the reason
 * atom). `ctx` supplies the other documented placeholders so templates
 * referencing `{session_id}`/`{turn_id}`/`{model}` still resolve. Returns `null`
 * when no enabled config or template applies.
 */
export function resolveGenerationTraceUrl(
  metadata: unknown,
  traceByDriver: Map<DriverId, ProviderTraceConfig>,
  ctx: { sessionId?: string; turnId?: string },
): string | null {
  if (!isRecord(metadata)) return null;
  const provider =
    typeof metadata.provider === "string" ? (metadata.provider as DriverId) : undefined;
  if (!provider) return null;
  const config = traceByDriver.get(provider);
  if (!config?.enabled || !config.generation_url_template) return null;
  return buildTraceUrl(config.generation_url_template, {
    responseId: typeof metadata.response_id === "string" ? metadata.response_id : undefined,
    model: typeof metadata.model === "string" ? metadata.model : undefined,
    sessionId: ctx.sessionId,
    turnId: ctx.turnId,
  });
}

/**
 * Resolve a link to the session's grouped trace for the given driver.
 */
export function resolveSessionTraceUrl(
  providerType: DriverId | undefined,
  sessionId: string,
  traceByDriver: Map<DriverId, ProviderTraceConfig>,
  opts?: { model?: string },
): string | null {
  if (!providerType) return null;
  const config = traceByDriver.get(providerType);
  if (!config?.enabled || !config.session_url_template) return null;
  return buildTraceUrl(config.session_url_template, { sessionId, model: opts?.model });
}
