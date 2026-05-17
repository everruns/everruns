// Query key factories for React Query
//
// Centralizes all query keys in one place for:
// - Consistent key structure across the app
// - Easy refactoring when keys need to change
// - Type-safe key generation
// - Hierarchical invalidation (e.g., invalidate all agent queries)
//
// Usage:
//   import { queryKeys } from "@/lib/query-keys";
//   useQuery({ queryKey: queryKeys.agents.list() });
//   queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });

export const queryKeys = {
  // Agent queries
  agents: {
    all: ["agents"] as const,
    list: (includeArchived = false) => ["agents", { includeArchived }] as const,
    detail: (agentId: string) => ["agent", agentId] as const,
    stats: (org?: string, agentId?: string) => ["agent", org, agentId, "stats"] as const,
    versions: (org?: string, agentId?: string) => ["agent", org, agentId, "versions"] as const,
    versionDiff: (org?: string, agentId?: string, from?: string, to?: string) =>
      ["agent", org, agentId, "versions", "diff", from, to] as const,
  },

  // Agent example queries
  agentExamples: {
    all: ["agent-examples"] as const,
    list: () => ["agent-examples"] as const,
  },

  // Harness queries
  harnesses: {
    all: ["harnesses"] as const,
    list: (includeArchived = false) => ["harnesses", { includeArchived }] as const,
    detail: (harnessId: string) => ["harness", harnessId] as const,
    stats: (org?: string, harnessId?: string) => ["harness", org, harnessId, "stats"] as const,
  },

  // Harness example queries
  harnessExamples: {
    all: ["harness-examples"] as const,
    list: () => ["harness-examples"] as const,
  },

  // Session queries (sessions are org-level, with optional agent filter)
  sessions: {
    all: () => ["sessions"] as const,
    list: (org?: string, agentId?: string, offset?: number, limit?: number) =>
      ["sessions", org, agentId ?? "all", offset ?? 0, limit ?? 20] as const,
    byAgent: (agentId: string) => ["sessions", "agent", agentId] as const,
    detail: (org?: string, sessionId?: string) => ["session", org, sessionId] as const,
    contextReport: (org?: string, sessionId?: string) =>
      ["session", org, sessionId, "context-report"] as const,
    stats: (org?: string) => ["sessions", "stats", org] as const,
  },

  // Event queries (events are session-level, no longer need agentId)
  events: {
    all: ["events"] as const,
    list: (sessionId: string) => ["events", sessionId] as const,
  },

  notifications: {
    all: ["notifications"] as const,
    list: () => ["notifications"] as const,
  },

  reporting: {
    all: ["reporting"] as const,
    catalog: (org?: string) => ["reporting", org, "catalog"] as const,
    query: (org?: string, query?: Record<string, unknown>) =>
      ["reporting", org, "query", query] as const,
    saved: (org?: string) => ["reporting", org, "saved"] as const,
    diagnostics: (org?: string) => ["reporting", org, "diagnostics"] as const,
  },

  // LLM Provider queries
  llmProviders: {
    all: ["llm-providers"] as const,
    list: () => ["llm-providers"] as const,
    detail: (providerId: string) => ["llm-providers", providerId] as const,
    models: (providerId: string) => ["llm-providers", providerId, "models"] as const,
  },

  // LLM Model queries
  llmModels: {
    all: ["llm-models"] as const,
    list: () => ["llm-models"] as const,
    detail: (modelId: string) => ["llm-models", modelId] as const,
  },

  // Session files queries
  sessionFiles: {
    all: ["session-files"] as const,
    list: (sessionId: string, path?: string) => ["session-files", sessionId, path ?? "/"] as const,
    detail: (sessionId: string, path: string) => ["session-file", sessionId, path] as const,
  },

  // User queries
  users: {
    all: ["users"] as const,
    list: () => ["users"] as const,
    detail: (userId: string) => ["user", userId] as const,
    me: () => ["user", "me"] as const,
    apiKeys: () => ["user", "api-keys"] as const,
  },

  // Auth queries
  auth: {
    session: () => ["auth", "session"] as const,
  },

  // Capability queries
  capabilities: {
    all: ["capabilities"] as const,
    list: () => ["capabilities"] as const,
    detail: (capabilityId: string) => ["capability", capabilityId] as const,
    available: () => ["capabilities", "available"] as const,
  },

  declarativeCapabilities: {
    all: ["declarative-capabilities"] as const,
    list: () => ["declarative-capabilities"] as const,
    detail: (capabilityId: string) => ["declarative-capability", capabilityId] as const,
  },

  // Durable queries
  durable: {
    workers: {
      all: ["durable-workers"] as const,
      list: () => ["durable-workers"] as const,
    },
    workflows: {
      all: ["durable-workflows"] as const,
      list: () => ["durable-workflows"] as const,
      detail: (workflowId: string) => ["durable-workflow", workflowId] as const,
    },
    runnable: {
      all: ["durable-runnable"] as const,
      list: () => ["durable-runnable"] as const,
    },
  },

  // MCP Server queries
  mcpServers: {
    all: ["mcp-servers"] as const,
    list: (includeArchived = false) => ["mcp-servers", { includeArchived }] as const,
    detail: (serverId: string) => ["mcp-server", serverId] as const,
  },

  // Organization queries
  organizations: {
    all: ["organizations"] as const,
    detail: (orgId: string) => ["organization", orgId] as const,
    members: (orgId: string) => ["organization", orgId, "members"] as const,
  },

  // Agent Identity Connection queries
  identityConnections: {
    all: ["identity-connections"] as const,
    list: (identityId: string) => ["identity-connections", identityId] as const,
  },

  // User Connection queries
  userConnections: {
    all: ["user-connections"] as const,
    list: () => ["user-connections"] as const,
  },

  payments: {
    all: ["payments"] as const,
    accounts: (params: Record<string, unknown> = {}) => ["payments", "accounts", params] as const,
    policies: (params: Record<string, unknown> = {}) => ["payments", "policies", params] as const,
    attempts: (params: Record<string, unknown> = {}) => ["payments", "attempts", params] as const,
  },

  // Session schedule queries
  sessionSchedules: {
    all: ["session-schedules"] as const,
    list: (sessionId: string) => ["session-schedules", sessionId] as const,
    detail: (sessionId: string, scheduleId: string) =>
      ["session-schedule", sessionId, scheduleId] as const,
  },

  sessionResources: {
    all: ["session-resources"] as const,
    list: (sessionId: string) => ["session-resources", sessionId] as const,
  },

  // Command queries
  commands: {
    all: ["commands"] as const,
    list: (sessionId: string) => ["commands", sessionId] as const,
  },

  // App queries
  apps: {
    all: ["apps"] as const,
    list: (includeArchived = false) => ["apps", { includeArchived }] as const,
    detail: (appId: string) => ["app", appId] as const,
  },

  // Workspace Volume queries
  volumes: {
    all: ["volumes"] as const,
    list: (includeArchived = false, search = "") =>
      ["volumes", { includeArchived, search }] as const,
    detail: (volumeId: string) => ["volume", volumeId] as const,
  },

  // Memory store queries
  memoryStores: {
    all: ["memory-stores"] as const,
    list: () => ["memory-stores"] as const,
    detail: (storeId: string) => ["memory-store", storeId] as const,
    memories: (storeId: string, params: Record<string, unknown> = {}) =>
      ["memory-store", storeId, "memories", params] as const,
  },

  // Eval queries
  evals: {
    all: ["evals"] as const,
    list: (includeArchived = false) => ["evals", { includeArchived }] as const,
    detail: (evalId: string) => ["eval", evalId] as const,
    cases: (evalId: string) => ["eval", evalId, "cases"] as const,
    runs: (evalId: string) => ["eval", evalId, "runs"] as const,
    runDetail: (evalId: string, runId: string) => ["eval", evalId, "run", runId] as const,
  },

  // Policy queries
  policies: {
    config: (resource: string) => ["policies", resource] as const,
  },

  // Skill queries
  skills: {
    all: ["skills"] as const,
    list: (includeArchived = false) => ["skills", { includeArchived }] as const,
    detail: (skillId: string) => ["skill", skillId] as const,
    content: (skillId: string) => ["skill", skillId, "content"] as const,
    usage: () => ["skills", "usage"] as const,
  },
};
