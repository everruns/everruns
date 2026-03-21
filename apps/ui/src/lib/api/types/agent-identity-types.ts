// Agent Identity types

export type AgentIdentityStatus = "active" | "archived" | "deleted";

export interface AgentIdentity {
  id: string;
  name: string;
  description?: string | null;
  avatar_url?: string | null;
  locale?: string | null;
  timezone?: string | null;
  status: AgentIdentityStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface CreateAgentIdentityRequest {
  name: string;
  description?: string;
  avatar_url?: string;
  locale?: string;
  timezone?: string;
}

export interface UpdateAgentIdentityRequest {
  name?: string;
  description?: string | null;
  avatar_url?: string | null;
  locale?: string | null;
  timezone?: string | null;
  status?: AgentIdentityStatus;
}
