// Agent Examples API — read-only examples adoptable as real Agents

import { api } from "./client";
import type { Agent, AgentExample } from "./types";

export async function listAgentExamples(): Promise<GuidedAgentExample[]> {
  const response = await api.get<GuidedAgentExample[]>("/v1/agent-examples");
  return response.data;
}

export async function importAgentExample(name: string): Promise<Agent> {
  const response = await api.post<Agent>(
    `/v1/agents/import?from-example=${encodeURIComponent(name)}`,
  );
  return response.data;
}

/// One yes/no setup choice; on sets `config_key` in the agent's `capability` config.
export interface AgentExampleSetting {
  key: string;
  label: string;
  description: string;
  capability: string;
  config_key: string;
  default: boolean;
}

/// Guided setup a template asks for after import (connect GitHub, pick a
/// repository, choose settings, create the trigger).
export interface AgentExampleSetup {
  connect_github: boolean;
  connections: string[];
  repository_placeholder: string;
  /// `POST /v1/agents/{id}/triggers` body once the placeholder is replaced.
  trigger: Record<string, unknown>;
  settings: AgentExampleSetting[];
}

export type GuidedAgentExample = Omit<AgentExample, "setup"> & { setup?: AgentExampleSetup };
