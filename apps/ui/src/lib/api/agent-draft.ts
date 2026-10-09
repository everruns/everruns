import { api } from "./client";

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
