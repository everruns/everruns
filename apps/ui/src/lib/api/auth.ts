// Auth API client functions
import { api, getBackendUrl } from "./client";
import type {
  AuthConfigResponse,
  LoginRequest,
  RegisterRequest,
  TokenResponse,
  UserInfoResponse,
  ApiKeyListItem,
  ApiKeyResponse,
  CreateApiKeyRequest,
} from "./types";

/**
 * Get authentication configuration from the server.
 * This tells the UI what auth mode is enabled and what options are available.
 */
export async function getAuthConfig(): Promise<AuthConfigResponse> {
  const { data } = await api.get<AuthConfigResponse>("/v1/auth/config");
  return data;
}

/**
 * Login with email and password
 */
export async function login(request: LoginRequest): Promise<TokenResponse> {
  const { data } = await api.post<TokenResponse>("/v1/auth/login", request);
  return data;
}

/**
 * Register a new user
 */
export async function register(
  request: RegisterRequest
): Promise<TokenResponse> {
  const { data } = await api.post<TokenResponse>("/v1/auth/register", request);
  return data;
}

/**
 * Get current user info
 */
export async function getCurrentUser(): Promise<UserInfoResponse> {
  const { data } = await api.get<UserInfoResponse>("/v1/auth/me");
  return data;
}

/**
 * Logout (clear cookies)
 */
export async function logout(): Promise<void> {
  await api.post<void>("/v1/auth/logout");
}

/**
 * Refresh the access token using the refresh token
 */
export async function refreshToken(token: string): Promise<TokenResponse> {
  const { data } = await api.post<TokenResponse>("/v1/auth/refresh", {
    refresh_token: token,
  });
  return data;
}

/**
 * Get OAuth redirect URL for a provider
 * Uses full URL since browser needs to navigate to this
 */
export function getOAuthUrl(provider: string): string {
  return `${getBackendUrl()}/v1/auth/oauth/${provider}`;
}

/**
 * List API keys for current user
 */
export async function listApiKeys(): Promise<ApiKeyListItem[]> {
  const { data } = await api.get<ApiKeyListItem[]>("/v1/auth/api-keys");
  return data;
}

/**
 * Create a new API key
 */
export async function createApiKey(
  request: CreateApiKeyRequest
): Promise<ApiKeyResponse> {
  const { data } = await api.post<ApiKeyResponse>("/v1/auth/api-keys", request);
  return data;
}

/**
 * Delete an API key
 */
export async function deleteApiKey(keyId: string): Promise<void> {
  await api.delete<void>(`/v1/auth/api-keys/${keyId}`);
}
