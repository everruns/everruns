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
 * - the result is not a safe public HTTPS URL. Templates come from org settings
 *   and are rendered as clickable `href`s, so reject plaintext and internal
 *   destinations to avoid leaking trace identifiers or targeting intranet hosts.
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

  return isSafeTraceUrl(url) ? url : null;
}

function isSafeTraceUrl(url: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }

  if (parsed.protocol !== "https:") return false;

  const hostname = parsed.hostname.toLowerCase().replace(/\.$/, "");
  if (hostname === "localhost" || hostname.endsWith(".localhost")) return false;
  if (hostname.endsWith(".local") || hostname.endsWith(".internal")) return false;

  return !isBlockedIpLiteral(hostname);
}

function isBlockedIpLiteral(hostname: string): boolean {
  const ipv4 = hostname.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
  if (ipv4) {
    const octets = ipv4.slice(1).map((part) => Number(part));
    if (octets.some((octet) => !Number.isInteger(octet) || octet < 0 || octet > 255)) {
      return true;
    }
    const [a, b, c] = octets;
    return (
      a === 0 ||
      a === 10 ||
      a === 127 ||
      (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && b === 168) ||
      (a === 169 && b === 254) ||
      (a === 100 && b >= 64 && b <= 127) ||
      (a === 192 && b === 0 && c === 2) ||
      (a === 198 && b === 51 && c === 100) ||
      (a === 203 && b === 0 && c === 113)
    );
  }

  const normalized = hostname.replace(/^\[|\]$/g, "");
  return (
    normalized === "::" ||
    normalized === "::1" ||
    normalized.startsWith("fe80:") ||
    normalized.toLowerCase().startsWith("::ffff:127.") ||
    normalized.toLowerCase().startsWith("::ffff:169.254.")
  );
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
