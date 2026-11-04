export interface PageResource {
  kind: string;
  id: string;
  label: string;
}

// Exact configuration endpoints only. Creates, deletes, credentials endpoints,
// operational actions and payments are never inferred from a URL prefix.
const definitions = [
  ["sandbox-templates", "sandbox_template", "Sandbox Template"],
  ["agents", "agent", "Agent"],
  ["harnesses", "harness", "Harness"],
  ["skills", "skill", "Skill"],
  ["capabilities/declarative", "capability", "Capability"],
  ["virtual-users", "virtual_user", "Virtual User"],
  ["observers", "observer", "Observer"],
  ["evals", "eval", "Eval"],
  ["models", "model", "Model"],
  ["providers", "provider", "Provider"],
  ["mcp-servers", "mcp_server", "MCP server"],
  ["knowledge-bases", "knowledge_base", "Knowledge base"],
  ["knowledge-indexes", "knowledge_index", "Knowledge index"],
  ["knowledge", "knowledge", "Knowledge collection"],
  ["memories", "memory", "Memory"],
  ["plugins", "plugin", "Plugin"],
  ["plugin_marketplaces", "plugin_marketplace", "Plugin marketplace"],
  ["workspaces", "workspace", "Workspace"],
  ["durable/schedules", "schedule", "Schedule"],
  ["reports/saved", "saved_report", "Report"],
  ["orgs", "organization", "Organization"],
] as const;

export function resourceWrite(
  method: string,
  endpoint: string,
  org?: string,
): PageResource | undefined {
  if (!["PATCH", "PUT"].includes(method.toUpperCase())) return;
  const path = endpoint.split("?")[0];
  if (path === "/v1/tracing/defaults" && org)
    return { kind: "tracing_policy", id: org, label: "Tracing" };
  const tracing = path.match(/^\/v1\/agents\/([^/]+)\/tracing$/);
  if (tracing) return { kind: "tracing_policy", id: tracing[1], label: "Tracing" };
  const destination = path.match(/^\/v1\/tracing\/destinations\/([^/]+)$/);
  if (destination)
    return { kind: "tracing_destination", id: destination[1], label: "Tracing destination" };
  const account = path.match(/^\/v1\/orgs\/[^/]+\/organization-connections\/([^/]+)$/);
  if (account)
    return { kind: "organization_account", id: account[1], label: "Organization account" };
  const network = path.match(/^\/v1\/orgs\/(org_[a-f0-9]+)\/egress-allowlist$/);
  if (network) return { kind: "organization_network", id: network[1], label: "Outbound allowlist" };
  if (path === "/v1/models/decision-default" && org)
    return { kind: "organization_decision", id: org, label: "Decision model" };
  for (const [prefix, kind, label] of definitions) {
    const root = `/v1/${prefix}/`;
    if (!path.startsWith(root)) continue;
    const id = path.slice(root.length);
    if (/^(?:[a-z][a-z_]*_)?[a-f0-9-]{8,}$/i.test(id)) return { kind, id, label };
  }
  const entry = path.match(/^\/v1\/(knowledge-bases|evals)\/[^/]+\/(entries|cases)\/([^/]+)$/);
  if (
    entry &&
    ((entry[1] === "knowledge-bases" && entry[2] === "entries") ||
      (entry[1] === "evals" && entry[2] === "cases"))
  )
    return {
      kind: entry[2] === "entries" ? "knowledge_entry" : "eval_case",
      id: entry[3],
      label: entry[2] === "entries" ? "Knowledge entry" : "Eval case",
    };
  const nested = path.match(/^\/v1\/agents\/[^/]+\/(channels|triggers|scripts)\/([^/]+)$/);
  if (nested)
    return {
      kind: { channels: "agent_channel", triggers: "agent_trigger", scripts: "agent_script" }[
        nested[1]
      ]!,
      id: nested[2],
      label: "Agent integration",
    };
  return;
}

export function formResource(type: string, id?: string, formId?: string): PageResource | undefined {
  if (!id || ["new", "builder", "page"].includes(id)) return;
  const aliases: Record<string, string> = {
    knowledge_collection: "knowledge",
    report: "saved_report",
    web_search_defaults: "organization",
    model_defaults: "organization",
    profile: "user_profile",
  };
  const kind =
    type === "model_defaults" && formId === "decision_model"
      ? "organization_decision"
      : (aliases[type] ?? type);
  if (kind === "tracing_policy") return { kind, id, label: "Tracing" };
  if (kind === "organization_account") return { kind, id, label: "Organization account" };
  if (kind === "user_profile") return { kind, id, label: "Account profile" };
  if (kind === "organization_network" || kind === "organization_decision")
    return {
      kind,
      id,
      label: kind === "organization_network" ? "Outbound allowlist" : "Decision model",
    };
  const definition = definitions.find((item) => item[1] === kind);
  if (definition) return { kind, id, label: definition[2] };
  if (
    ["agent_channel", "agent_trigger", "agent_script", "eval_case", "knowledge_entry"].includes(
      kind,
    )
  )
    return { kind, id, label: type.replaceAll("_", " ") };
}

export function resourceKey(resource: PageResource) {
  return `${resource.kind}:${resource.id}`;
}
