// API base URL configuration:
// All API requests (including SSE) use /api prefix.
// Caddy reverse proxy strips /api and forwards to backend in all environments.
const API_BASE = "/api";

// Org selection is handled via server-side cookie (everruns_org), set by
// POST /v1/users/me/switch-org. This works automatically with all requests
// including SSE (EventSource) because cookies are sent automatically.

export class ApiError extends Error {
  constructor(
    public status: number,
    public statusText: string,
    message?: string,
  ) {
    super(message || `API Error: ${status} ${statusText}`);
    this.name = "ApiError";
  }
}

// Token refresh state: deduplicates concurrent refresh attempts so multiple
// 401 responses don't each consume a refresh token.
let refreshPromise: Promise<boolean> | null = null;

async function tryRefreshToken(): Promise<boolean> {
  try {
    // POST with no body — backend reads refresh_token from HttpOnly cookie
    const response = await fetch(`${API_BASE}/v1/auth/refresh`, {
      method: "POST",
      credentials: "include",
    });
    return response.ok;
  } catch {
    return false;
  }
}

async function request<T>(endpoint: string, options: RequestInit = {}): Promise<{ data: T }> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(options.headers as Record<string, string>),
  };

  const doFetch = () =>
    fetch(`${API_BASE}${endpoint}`, {
      ...options,
      credentials: "include", // Include cookies for auth (access_token, everruns_org)
      headers,
    });

  let response = await doFetch();

  // On 401, try refreshing the access token once (skip for auth endpoints to avoid loops)
  if (response.status === 401 && !endpoint.startsWith("/v1/auth/")) {
    if (!refreshPromise) {
      refreshPromise = tryRefreshToken().finally(() => {
        refreshPromise = null;
      });
    }
    const refreshed = await refreshPromise;
    if (refreshed) {
      response = await doFetch();
    }
  }

  if (!response.ok) {
    // Try to get error details from response body
    let errorMessage: string | undefined;
    try {
      const errorBody = await response.json();
      errorMessage = errorBody.error || errorBody.message || JSON.stringify(errorBody);
    } catch {
      // Response body is not JSON or empty
    }
    throw new ApiError(response.status, response.statusText, errorMessage);
  }

  // Handle empty responses (204 No Content or empty body)
  if (response.status === 204) {
    return { data: {} as T };
  }

  // Check if response has content before parsing JSON
  const text = await response.text();
  if (!text) {
    return { data: {} as T };
  }

  const data = JSON.parse(text);
  return { data };
}

// Axios-like API client
export const api = {
  defaults: {
    baseURL: API_BASE,
  },

  get: <T>(url: string) => request<T>(url, { method: "GET" }),

  post: <T>(url: string, body?: unknown) =>
    request<T>(url, {
      method: "POST",
      body: body ? JSON.stringify(body) : undefined,
    }),

  patch: <T>(url: string, body?: unknown) =>
    request<T>(url, {
      method: "PATCH",
      body: body ? JSON.stringify(body) : undefined,
    }),

  put: <T>(url: string, body?: unknown) =>
    request<T>(url, {
      method: "PUT",
      body: body ? JSON.stringify(body) : undefined,
    }),

  delete: <T>(url: string) => request<T>(url, { method: "DELETE" }),
};

/**
 * Extract an ApiError from a failed fetch Response.
 * Use this in callsites that need raw fetch() (FormData, text responses,
 * header access) but still want centralized error handling.
 */
export async function throwApiError(response: Response): Promise<never> {
  let errorMessage: string | undefined;
  try {
    const errorBody = await response.json();
    errorMessage = errorBody.error || errorBody.message || JSON.stringify(errorBody);
  } catch {
    try {
      errorMessage = await response.text();
    } catch {
      // No body at all
    }
  }
  throw new ApiError(response.status, response.statusText, errorMessage);
}

export function getApiBaseUrl(): string {
  return API_BASE;
}

/**
 * Get the direct backend URL for operations that can't go through the proxy
 * (e.g., OAuth redirects that need browser navigation to the backend)
 */
export function getBackendUrl(): string {
  // In browser, use window.location to construct the proxy URL
  // OAuth will go through /api/v1/auth/oauth which gets proxied
  if (typeof window !== "undefined") {
    return window.location.origin + API_BASE;
  }
  return API_BASE;
}
