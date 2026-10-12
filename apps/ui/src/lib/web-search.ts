// Web search in the UI: the shapes the Web search page, the agent editor and
// the chat row read, and the plain-words helpers they share.
//
// Decision: the chat row and the Trace read the people-only `details` of the
// `web_search` tool result (crates/integrations/src/web_search/tool.rs), never
// the model-facing result, so they can show every attempt and raw response
// whatever `show_provider` says. Anything not shaped like those details is
// ignored and the row falls back to the generic tool row.
// See knowledge/integrations/web-search.md (UI, Details view and Session Trace).

import type {
  SearchAccountSummary,
  SearchPlanPreview,
  SearchProviderSetting,
  WebSearchDefaults,
} from "@/lib/api/types";

export const WEB_SEARCH_TOOL_NAME = "web_search";
export const WEB_SEARCH_CAPABILITY_ID = "web_search";
export const AUTOMATIC_NATIVE = "native";
export const AUTOMATIC_FREE = "free";
/** Mirrors `WEB_SEARCH_MAX_RESULTS` / `WEB_SEARCH_DEFAULT_RESULTS` in everruns-contracts. */
export const WEB_SEARCH_MAX_RESULTS = 20;
export const WEB_SEARCH_DEFAULT_RESULTS = 8;
/** Short form of `NATIVE_WEB_SEARCH_PRICE_NOTE` for badges. */
export const NATIVE_PRICE_SHORT = "≈ $10 / 1k";

export const SETTING_LABELS: Record<SearchProviderSetting, string> = {
  org_default: "Organization default",
  auto: "Automatic",
  native: "Model provider search",
  brave: "Brave Search",
  parallel: "Parallel",
  account: "Search account",
  free: "Free search",
};

export type SearchFailureKind =
  | "not_configured"
  | "unauthorized"
  | "rate_limited"
  | "blocked"
  | "network_denied"
  | "unavailable";

export interface SearchExchange {
  method: string;
  url: string;
  status?: number;
  body?: string;
  truncated?: boolean;
  error?: string;
}

export interface SearchAttempt {
  provider: string;
  label: string;
  ok: boolean;
  duration_ms: number;
  result_count?: number;
  failure_kind?: SearchFailureKind;
  message?: string;
  notes?: string[];
  exchanges?: SearchExchange[];
}

export interface SearchResultEntry {
  id: string;
  title: string;
  url: string;
  site: string;
  published?: string;
  snippet?: string;
  excerpt?: string;
}

export interface WebSearchDetails {
  kind: "web_search";
  query: string;
  setting?: { provider?: string; account_id?: string | null; fallback?: string | null };
  resolved?: { provider?: string; account_id?: string | null; fallback?: string | null };
  plan?: string[];
  ok: boolean;
  attempts: SearchAttempt[];
  results: SearchResultEntry[];
  provider?: string;
  provider_label?: string;
  answer?: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The `details` of a `web_search` tool result, or null for anything else. */
export function parseWebSearchDetails(details: unknown): WebSearchDetails | null {
  if (!isRecord(details) || details.kind !== "web_search") return null;
  if (typeof details.query !== "string") return null;
  const attempts = Array.isArray(details.attempts)
    ? details.attempts.filter(
        (attempt): attempt is SearchAttempt =>
          isRecord(attempt) && typeof attempt.provider === "string",
      )
    : [];
  const results = Array.isArray(details.results)
    ? details.results.filter(
        (result): result is SearchResultEntry => isRecord(result) && typeof result.url === "string",
      )
    : [];
  return {
    ...(details as unknown as WebSearchDetails),
    ok: details.ok !== false,
    attempts,
    results,
  };
}

const FAILURE_PHRASES: Record<SearchFailureKind, string> = {
  not_configured: "is not connected",
  unauthorized: "rejected the key",
  rate_limited: "was rate limited",
  blocked: "was blocked",
  network_denied: "is blocked by network policy",
  unavailable: "was unavailable",
};

/** "Brave Search was rate limited". */
export function attemptFailurePhrase(attempt: SearchAttempt): string {
  const phrase = attempt.failure_kind ? FAILURE_PHRASES[attempt.failure_kind] : "failed";
  return `${attempt.label} ${phrase ?? "failed"}`;
}

/** Short status of one attempt for the attempts list: "429 rate limited", "4 results". */
export function attemptStatus(attempt: SearchAttempt): string {
  if (attempt.ok) {
    const count = attempt.result_count ?? 0;
    return `${count} ${count === 1 ? "result" : "results"}`;
  }
  const status = attempt.exchanges?.find((exchange) => exchange.status)?.status;
  const kind = attempt.failure_kind?.replace(/_/g, " ") ?? "failed";
  // Nothing ran (no account, model without search): say it was skipped.
  if (attempt.duration_ms === 0 && !attempt.exchanges?.length) {
    return attempt.message ? `skipped · ${attempt.message}` : "skipped";
  }
  return status ? `${status} ${kind}` : kind;
}

export interface WebSearchSummary {
  query: string;
  resultCount: number;
  /** Who answered, or null when nobody did. */
  providerLabel: string | null;
  durationMs: number;
  /** "Brave Search was rate limited", when an earlier provider failed. */
  fallbackNote: string | null;
  failed: boolean;
  failureReason: string | null;
}

/** What the chat row says about one search. */
export function summarizeWebSearch(
  details: WebSearchDetails,
  durationMs?: number | null,
): WebSearchSummary {
  // Skipped steps (nothing ran) are not fallbacks worth a note in the row.
  const failedBefore = details.attempts.filter(
    (attempt) => !attempt.ok && (attempt.duration_ms > 0 || attempt.exchanges?.length),
  );
  const last = details.attempts[details.attempts.length - 1];
  return {
    query: details.query,
    resultCount: details.results.length,
    providerLabel: details.ok ? (details.provider_label ?? last?.label ?? null) : null,
    durationMs:
      durationMs ?? details.attempts.reduce((total, attempt) => total + attempt.duration_ms, 0),
    fallbackNote:
      details.ok && failedBefore.length > 0
        ? failedBefore.map(attemptFailurePhrase).join(", ")
        : null,
    failed: !details.ok,
    failureReason: details.ok ? null : (last?.message ?? "no provider answered"),
  };
}

/** "Organization default · Automatic". */
export function settingLabel(
  details: WebSearchDetails,
  accounts: { id: string; name: string }[] = [],
): string {
  const label = (provider?: string, accountId?: string | null) => {
    if (provider === "account" && accountId) {
      const account = accounts.find((candidate) => candidate.id === accountId);
      return account ? account.name : SETTING_LABELS.account;
    }
    return SETTING_LABELS[provider as SearchProviderSetting] ?? provider ?? "Unknown";
  };
  const setting = details.setting?.provider;
  const resolved = details.resolved?.provider;
  if (setting === "org_default" && resolved && resolved !== "org_default") {
    return `${SETTING_LABELS.org_default} · ${label(resolved, details.resolved?.account_id)}`;
  }
  return label(setting ?? resolved, details.setting?.account_id ?? details.resolved?.account_id);
}

export function formatSearchDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Host of a URL without `www.`, for result lists. */
export function siteOf(result: SearchResultEntry): string {
  if (result.site) return result.site;
  try {
    return new URL(result.url).hostname.replace(/^www\./, "");
  } catch {
    return result.url;
  }
}

// ---------------------------------------------------------------------------
// Organization defaults: the Automatic order
// ---------------------------------------------------------------------------

export interface AutomaticEntry {
  /** `native`, an account id, or `free`. */
  id: string;
  kind: "native" | "account" | "free";
  label: string;
  /** Native is off, or the account has no key: listed, but not used. */
  inactive: boolean;
  note?: string;
}

/**
 * The Automatic order as the Defaults tab lists it. Mirrors the plan
 * resolution: the stored order first (unknown accounts dropped), then entries
 * it does not name in their natural place (native first, unlisted accounts
 * after the listed ones), free search always last.
 */
export function automaticEntries(
  defaults: WebSearchDefaults,
  accounts: SearchAccountSummary[],
): AutomaticEntry[] {
  const allowNative = defaults.allow_native !== false;
  const native: AutomaticEntry = {
    id: AUTOMATIC_NATIVE,
    kind: "native",
    label: "Model provider search",
    inactive: !allowNative,
    note: allowNative ? undefined : "turned off",
  };
  const accountEntry = (account: SearchAccountSummary): AutomaticEntry => ({
    id: account.id,
    kind: "account",
    label: accountLabel(account),
    inactive: account.status !== "ready",
    note: account.status === "ready" ? undefined : "needs a key",
  });
  const order = (defaults.automatic_order ?? [AUTOMATIC_NATIVE, AUTOMATIC_FREE]).filter(
    (id) => id !== AUTOMATIC_FREE,
  );
  const listed: AutomaticEntry[] = [];
  for (const id of order) {
    if (id === AUTOMATIC_NATIVE) listed.push(native);
    else {
      const account = accounts.find((candidate) => candidate.id === id);
      if (account) listed.push(accountEntry(account));
    }
  }
  if (!listed.some((entry) => entry.kind === "native")) listed.unshift(native);
  for (const account of accounts) {
    if (!listed.some((entry) => entry.id === account.id)) listed.push(accountEntry(account));
  }
  listed.push({ id: AUTOMATIC_FREE, kind: "free", label: "Free search", inactive: false });
  return listed;
}

/** Move one entry up (-1) or down (+1); free search stays last. */
export function moveAutomaticEntry(entries: AutomaticEntry[], index: number, delta: -1 | 1) {
  const target = index + delta;
  const last = entries.length - 1;
  if (index < 0 || index >= last || target < 0 || target >= last) return entries;
  const next = [...entries];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

/** The order to store: every entry, free search last. */
export function automaticOrder(entries: AutomaticEntry[]): string[] {
  return [...entries.filter((entry) => entry.kind !== "free").map((entry) => entry.id), "free"];
}

const CONNECTOR_NAMES: Record<string, string> = {
  brave_search: "Brave Search",
  parallel: "Parallel",
  cloudflare_web_search: "Cloudflare Web Search",
};

/** "Brave Search · Team". */
export function accountLabel(account: Pick<SearchAccountSummary, "provider" | "name">): string {
  const provider = CONNECTOR_NAMES[account.provider] ?? account.provider;
  return account.name && account.name !== account.provider
    ? `${provider} · ${account.name}`
    : provider;
}

// ---------------------------------------------------------------------------
// Agent editor: "How this agent will search"
// ---------------------------------------------------------------------------

/**
 * Plain words for a previewed plan: which provider answers first for this
 * model, what it costs, and what happens when it cannot.
 */
export function describePlan(preview: SearchPlanPreview, modelName?: string): string[] {
  const steps = preview.steps;
  const usable = steps.filter((step) => step.available);
  const lines: string[] = [];
  const model = modelName ? `With ${modelName}` : "With the default model";
  const first = usable[0];
  if (!first) {
    lines.push(`${model}: no provider can answer, so searches fail.`);
    return lines;
  }
  const cost = first.price_note ? ` (${lowerFirst(first.price_note)})` : "";
  const rest = usable.slice(1).map((step) => step.label);
  lines.push(`${model}: ${first.label}${cost} answers first.`);
  if (preview.fallback === "none") {
    lines.push("When it cannot answer, the search fails: fallback is off.");
  } else if (rest.length > 0) {
    lines.push(`When it cannot answer: ${joinWords(rest)}.`);
  }
  const native = steps.find((step) => step.id === "native");
  if (native && !native.available && native.note && preview.provider !== "free") {
    lines.push(`Model provider search is skipped: ${lowerFirst(native.note)}.`);
  }
  lines.push(`${preview.limit} results per search unless the agent asks for fewer.`);
  return lines;
}

function lowerFirst(text: string): string {
  return text ? text.charAt(0).toLowerCase() + text.slice(1) : text;
}

function joinWords(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")}, then ${items[items.length - 1]}`;
}

/** Split a comma list into trimmed, de-duplicated domains. */
export function parseDomainList(value: string): string[] {
  const out: string[] = [];
  for (const part of value.split(",")) {
    const domain = part.trim().toLowerCase();
    if (domain && !out.includes(domain)) out.push(domain);
  }
  return out;
}

/** A bare host name, as the server accepts it. */
export function isValidDomain(domain: string): boolean {
  return domain.length > 0 && !/[/\s:]/.test(domain);
}
