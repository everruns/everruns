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
    list: () => ["agents"] as const,
    detail: (agentId: string) => ["agent", agentId] as const,
  },

  // Session queries
  sessions: {
    all: ["sessions"] as const,
    list: (agentId: string) => ["sessions", agentId] as const,
    detail: (agentId: string, sessionId: string) =>
      ["session", agentId, sessionId] as const,
  },

  // Event queries
  events: {
    all: ["events"] as const,
    list: (agentId: string, sessionId: string) =>
      ["events", agentId, sessionId] as const,
  },

  // LLM Provider queries
  llmProviders: {
    all: ["llm-providers"] as const,
    list: () => ["llm-providers"] as const,
    detail: (providerId: string) => ["llm-providers", providerId] as const,
    models: (providerId: string) =>
      ["llm-providers", providerId, "models"] as const,
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
    list: (sessionId: string, path?: string) =>
      ["session-files", sessionId, path ?? "/"] as const,
    detail: (sessionId: string, path: string) =>
      ["session-file", sessionId, path] as const,
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
};
