// Auth API client functions
import { api, getBackendUrl } from "./client";
import type {
  AuthConfigResponse,
  LoginRequest,
  RegisterRequest,
  TokenResponse,
  UserInfoResponse,
  PersonalAccessTokenListItem,
  PersonalAccessTokenResponse,
  CreatePersonalAccessTokenRequest,
  ListResponse,
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
export async function register(request: RegisterRequest): Promise<TokenResponse> {
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
 * Refresh the access token using the refresh token cookie.
 * The HttpOnly refresh_token cookie is sent automatically by the browser.
 */
export async function refreshToken(): Promise<TokenResponse> {
  const { data } = await api.post<TokenResponse>("/v1/auth/refresh");
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
 * List personal access tokens for current user
 */
export async function listPersonalAccessTokens(): Promise<PersonalAccessTokenListItem[]> {
  const { data } = await api.get<ListResponse<PersonalAccessTokenListItem>>(
    "/v1/auth/personal-access-tokens",
  );
  return data.data;
}

/**
 * Create a new personal access token
 */
export async function createPersonalAccessToken(
  request: CreatePersonalAccessTokenRequest,
): Promise<PersonalAccessTokenResponse> {
  const { data } = await api.post<PersonalAccessTokenResponse>(
    "/v1/auth/personal-access-tokens",
    request,
  );
  return data;
}

/**
 * Delete a personal access token
 */
export async function deletePersonalAccessToken(tokenId: string): Promise<void> {
  await api.delete<void>(`/v1/auth/personal-access-tokens/${tokenId}`);
}
