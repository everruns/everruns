import { api } from "./client";
import type { McpServerActsAs } from "./schema-types";

export type DraftRole = "user" | "assistant";

export interface DraftTurn {
  role: DraftRole;
  content: string;
}

export interface DraftSchedule {
  /** Five-field cron expression. */
  cron: string;
  timezone: string;
  message: string;
}

/** GitHub events that start the agent; created once GitHub is connected on it. */
export interface DraftGitHubTrigger {
  events: string[];
  /** `owner/name`; empty means every repository the App sees. */
  repositories: string[];
  /** Run only when a CI outcome changes. */
  changes_only: boolean;
  /** CI events only: branches to accept; empty means every branch. */
  branches?: string[];
  /** CI events only: workflow names to accept; empty means every workflow. */
  workflows?: string[];
  message: string;
}

/** An org catalog MCP server to attach, with the grant its auth needs. */
export interface DraftMcpServer {
  name: string;
  acts_as: McpServerActsAs;
}

/** Channel kinds the builder may propose; each becomes a draft channel. */
export type DraftChannelKind = "public_chat" | "ag_ui" | "webhook" | "slack";

export interface AgentDraft {
  display_name: string;
  name: string;
  description: string;
  system_prompt: string;
  schedule?: DraftSchedule | null;
  capabilities: string[];
  channels: DraftChannelKind[];
  github_trigger?: DraftGitHubTrigger | null;
  mcp_servers?: DraftMcpServer[];
  /** Set by the server only when the org default model has no working key. */
  model_id?: string | null;
}

export interface AgentDraftResult {
  reply: string;
  draft: AgentDraft;
  suggestions: string[];
}

export const EMPTY_DRAFT: AgentDraft = {
  display_name: "",
  name: "",
  description: "",
  system_prompt: "",
  schedule: null,
  capabilities: [],
  channels: [],
  github_trigger: null,
  mcp_servers: [],
  model_id: null,
};

/** Ask the agent builder to revise a draft. Creates nothing. */
export async function draftAgent(
  messages: DraftTurn[],
  draft: AgentDraft,
): Promise<AgentDraftResult> {
  const response = await api.post<AgentDraftResult>("/v1/agents/draft", {
    messages,
    draft,
  });
  return response.data;
}
