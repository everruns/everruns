import type { PrincipalSummary } from "./legacy-api-types";

// Virtual User types
export type VirtualUserStatus = "active" | "archived" | "deleted";

export interface VirtualUser {
  id: string;
  organization_id: string;
  usage: "end_user" | "service";
  name: string;
  description?: string | null;
  avatar_url?: string | null;
  locale?: string | null;
  timezone?: string | null;
  principal?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
  status: VirtualUserStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface CreateVirtualUserRequest {
  usage?: "end_user" | "service";
  name: string;
  description?: string;
  avatar_url?: string;
  locale?: string;
  timezone?: string;
}

export interface UpdateVirtualUserRequest {
  name?: string;
  description?: string | null;
  avatar_url?: string | null;
  locale?: string | null;
  timezone?: string | null;
  status?: VirtualUserStatus;
}
