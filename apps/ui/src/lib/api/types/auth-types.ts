// Authentication, user, and organization types

// ============================================
// Authentication types
// ============================================

export type AuthMode = "none" | "admin" | "full" | "external";

export interface FeatureFlags {
  global_chat: boolean;
  apps: boolean;
  notifications: boolean;
}

export interface AuthConfigResponse {
  mode: AuthMode;
  password_auth_enabled: boolean;
  oauth_providers: string[];
  signup_enabled: boolean;
}

/** Response from GET /v1/{resource}/config */
export interface ResourceConfigResponse {
  policies: Record<string, boolean>;
}

export interface LoginRequest {
  email: string;
  password: string;
}

export interface RegisterRequest {
  email: string;
  password: string;
  name: string;
}

export interface TokenResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token?: string;
}

/** Organization role */
export type OrgRole = "owner" | "admin" | "member";

/** Organization membership info */
export interface OrganizationMembership {
  public_id: string;
  name: string;
  role: OrgRole;
}

/** Full organization details (from API) */
export interface Organization {
  id: string;
  name: string;
  default_model_id: string | null;
  default_harness_id: string | null;
  base_harness_id: string | null;
  created_at: string;
  updated_at: string;
}

/** Request to create an organization */
export interface CreateOrganizationRequest {
  name: string;
}

/** Request to update an organization */
export interface UpdateOrganizationRequest {
  name?: string;
  default_model_id?: string;
  default_harness_id?: string;
  base_harness_id?: string;
}

export interface UserInfoResponse {
  id: string;
  email: string;
  name: string;
  roles: string[];
  avatar_url?: string;
  /** Organizations the user belongs to */
  organizations?: OrganizationMembership[];
}

export interface ApiKeyResponse {
  id: string;
  name: string;
  key: string;
  key_prefix: string;
  scopes: string[];
  expires_at?: string;
  created_at: string;
}

export interface ApiKeyListItem {
  id: string;
  name: string;
  key_prefix: string;
  scopes: string[];
  expires_at?: string;
  last_used_at?: string;
  created_at: string;
}

export interface CreateApiKeyRequest {
  name: string;
  scopes?: string[];
  expires_in_days?: number;
}

export interface RefreshTokenRequest {
  refresh_token: string;
}

// ============================================
// User types (for members management)
// ============================================

export interface User {
  id: string;
  email: string;
  name: string;
  avatar_url?: string;
  roles: string[];
  auth_provider?: string;
  created_at: string;
}

export interface ListUsersQuery {
  search?: string;
}

/** Request to update current user profile */
export interface UpdateProfileRequest {
  name: string;
}

/** Response from profile update */
export interface ProfileResponse {
  id: string;
  email: string;
  name: string;
  avatar_url?: string;
}

// ============================================
// User Connection types
// ============================================

export interface UserConnection {
  provider: string;
  connection_type: string;
  provider_username?: string;
  scopes?: string;
  connected_at: string;
}

export interface ConnectionProvider {
  provider_id: string;
  display_name: string;
  description: string;
  icon: string;
  connection_type: "oauth" | "api_key";
  form_schema?: ConnectionFormSchema;
}

export interface ConnectionFormSchema {
  fields: ConnectionFormField[];
  instructions_markdown: string;
}

export interface ConnectionFormField {
  name: string;
  label: string;
  field_type: "password" | "text" | "url";
  required: boolean;
  placeholder?: string;
  help_text?: string;
}

export interface VerifyConnectionResponse {
  valid: boolean;
  error?: string;
}
