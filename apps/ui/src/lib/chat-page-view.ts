import type { ChatPageContext } from "@/components/chat-mode/chat-mode-context";
import { defaultNavigationSections } from "./navigation";
import { settingsNavigationSections } from "./settings-navigation";

const resources: Record<string, { type: string; roots: string[] }> = {
  dashboard: { type: "dashboard", roots: ["sessions", "agents", "harnesses"] },
  // Home registers its attention rows explicitly; the cache fallback covers its lists.
  home: { type: "home", roots: ["agents", "sessions", "sandboxes"] },
  approvals: { type: "approval", roots: [] },
  exposures: { type: "exposure", roots: [] },
  agents: {
    type: "agent",
    roots: ["agents", "agent", "agent-channels", "agent-triggers", "agent-examples"],
  },
  harnesses: { type: "harness", roots: ["harnesses", "harness", "harness-examples"] },
  sessions: { type: "session", roots: ["sessions", "session"] },
  playground: { type: "session", roots: ["playground", "sessions", "session"] },
  chats: { type: "session", roots: ["chat-threads", "session"] },
  models: { type: "model", roots: ["models", "providers", "model-profiles", "decision-default"] },
  "web-search": {
    type: "web_search",
    roots: ["web-search", "web-search-accounts", "web-search-defaults"],
  },
  "virtual-users": { type: "virtual_user", roots: ["identities", "identity", "virtual-users"] },
  knowledge: {
    type: "knowledge_collection",
    roots: ["knowledge", "knowledge-collections", "knowledge-indexes", "knowledge-indexers"],
  },
  "knowledge-indexes": { type: "knowledge_index", roots: ["knowledge-indexes"] },
  memory: { type: "memory", roots: ["memory"] },
  skills: { type: "skill", roots: ["skills", "skill"] },
  capabilities: {
    type: "capability",
    roots: ["capabilities", "capability", "declarative-capabilities"],
  },
  plugins: { type: "plugin", roots: ["plugins"] },
  sandboxes: { type: "sandbox", roots: ["sandboxes", "sandbox-templates", "sandbox-accounts"] },
  evals: { type: "eval", roots: ["evals", "eval", "eval-runs", "eval-run"] },
  observers: { type: "observer", roots: ["observers", "observer"] },
  reports: { type: "report", roots: ["reporting"] },
  monitoring: { type: "monitoring", roots: ["monitoring"] },
  durable: { type: "durable", roots: ["durable"] },
  settings: { type: "settings", roots: [] },
  // Installation-wide data is projected by its authorized page, never through
  // the tenant query-cache fallback.
  system: { type: "system", roots: [] },
  orgs: { type: "organization", roots: [] },
};
const reserved = new Set([
  "new",
  "all",
  "examples",
  "edit",
  "setup",
  "defaults",
  "providers",
  "accounts",
  "templates",
  "history",
  "declarative",
  "health",
  "workers",
  "workflows",
  "queues",
  "schedules",
  "circuit-breakers",
]);

export function pageLocation(route: string, label?: string): ChatPageContext {
  const segments = route.split("?")[0].split("/").filter(Boolean);
  const root = segments[0] ?? "";
  const resource = resources[root];
  const item = defaultNavigationSections
    .flatMap((section) => section.items)
    .find((item) => item.href.split("/")[1] === root);
  const settingsItem =
    root === "settings"
      ? settingsNavigationSections
          .flatMap((section) => section.items)
          .find((item) => item.href === route.split("?")[0])
      : undefined;
  const fallbackLabel =
    root === "settings" && segments[1]
      ? `${settingsItem?.name ?? segments[1].replaceAll("-", " ")} · Settings`
      : item?.name;
  const candidate =
    segments.at(-1) === "new"
      ? undefined
      : (segments.findLast((segment) =>
          /^(?:[a-z][a-z_]*_)?[0-9a-f]{8}-?[0-9a-f-]{20,}$/i.test(segment),
        ) ??
        (segments[1] && !reserved.has(segments[1]) && root !== "settings"
          ? segments[1]
          : undefined));
  const subtype =
    root === "models" && segments[1] === "providers"
      ? "provider"
      : root === "sandboxes" && segments[1] === "templates"
        ? "sandbox_template"
        : root === "sandboxes" && segments[1] === "accounts"
          ? "sandbox_account"
          : root === "agents" && segments[2] === "channels"
            ? "agent_channel"
            : root === "agents" && segments[2] === "triggers"
              ? "agent_trigger"
              : root === "evals" && segments[2] === "runs"
                ? "eval_run"
                : resource?.type;
  return {
    route,
    label:
      label || fallbackLabel || segments.map((s) => s.replaceAll("-", " ")).join(" / ") || "Home",
    ...(subtype ? { entity_type: subtype } : {}),
    ...(candidate ? { entity_id: candidate } : {}),
  };
}

// THREAT[TM-CLIENT-007]: never serialize a cache wholesale. Only active queries
// for this organization/resource qualify; projection omits credentials, config,
// request headers, conversation bodies, file contents and unknown fields.
const safeFields = new Set([
  "id",
  "name",
  "display_name",
  "description",
  "short_description",
  "status",
  "phase",
  "type",
  "kind",
  "state",
  "total",
  "count",
  "total_count",
  "updated_at",
  "created_at",
  "provider_type",
  "model_id",
  "agent_id",
  "harness_id",
  "parent_harness_id",
  "tags",
  "enabled",
  "is_enabled",
  "is_built_in",
  "version",
  "counts",
  "items",
  "data",
  "results",
  "pages",
  "total_pages",
  "page_size",
  "next_cursor",
]);
const privateKey =
  /^(?:credentials?|secrets?|api[-_]keys?|keys|tokens?|auth|config|events|messages|context-report|approval-audit)$/i;

export interface PageSummaryBudget {
  nodes: number;
  characters: number;
  truncated: boolean;
}
export function summarizePageData(
  value: unknown,
  depth = 0,
  budget: PageSummaryBudget = { nodes: 0, characters: 0, truncated: false },
): unknown {
  if (++budget.nodes > 2000 || budget.characters >= 16 * 1024 || depth > 6) {
    budget.truncated = true;
    return undefined;
  }
  if (value === null || typeof value === "boolean" || typeof value === "number") return value;
  if (typeof value === "string") {
    const text = value.slice(0, Math.min(512, 16 * 1024 - budget.characters));
    budget.characters += text.length;
    if (text.length < value.length) budget.truncated = true;
    return text;
  }
  if (Array.isArray(value)) {
    if (value.length > 50) budget.truncated = true;
    return value
      .slice(0, 50)
      .map((item) => summarizePageData(item, depth + 1, budget))
      .filter((item) => item !== undefined);
  }
  if (!value || typeof value !== "object") return undefined;
  return Object.fromEntries(
    [...safeFields]
      .filter((key) => Object.hasOwn(value, key))
      .map((key) => [
        key,
        summarizePageData((value as Record<string, unknown>)[key], depth + 1, budget),
      ])
      .filter(([, item]) => item !== undefined),
  );
}

export function isPageQuery(key: readonly unknown[], page: ChatPageContext, org: string): boolean {
  const root = page.route.split("/")[1]?.split("?")[0];
  const allowed = resources[root]?.roots ?? [];
  if (typeof key[0] !== "string" || !allowed.includes(key[0])) return false;
  // CRUD hooks append the org. Session/report/stat factories put it at a
  // documented position; a search term equal to an org id is not tenancy.
  const scoped =
    key.at(-1) === org ||
    ([
      "agent",
      "harness",
      "session",
      "sessions",
      "reporting",
      "model-profiles",
      "decision-default",
      "knowledge",
    ].includes(key[0]) &&
      key[1] === org) ||
    (key[0] === "sessions" && key[1] === "filtered" && key[2] === org);
  const virtualUserList = key[0] === "virtual-users" && key[1] === "list" && key[3] === org;
  if (!scoped && !virtualUserList) return false;
  if (key.some((part) => typeof part === "string" && privateKey.test(part))) return false;
  return !page.entity_id || key.includes(page.entity_id);
}
