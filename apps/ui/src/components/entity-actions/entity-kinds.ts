// Entity kinds the actions menu and record sheets know, and how prose names them.

/**
 * Entity kinds as the history API names them (`agent`, `harness`, ...). Only
 * kinds with their own detail page and manager notes; knowledge entries and
 * eval cases keep notes on their parent.
 */
export type EntityKind =
  | "agent"
  | "harness"
  | "skill"
  | "capability"
  | "knowledge_index"
  | "memory"
  | "provider"
  | "model"
  | "mcp_server"
  | "plugin"
  | "virtual_user"
  | "observer"
  | "eval"
  | "schedule";

const KIND_LABELS: Record<EntityKind, string> = {
  agent: "agent",
  harness: "harness",
  skill: "skill",
  capability: "capability",
  knowledge_index: "knowledge index",
  memory: "memory",
  provider: "provider",
  model: "model",
  mcp_server: "MCP server",
  plugin: "plugin",
  virtual_user: "virtual user",
  observer: "observer",
  eval: "eval",
  schedule: "schedule",
};

export function entityKindLabel(kind: EntityKind): string {
  return KIND_LABELS[kind];
}
