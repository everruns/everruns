// API base URL configuration:
// All API requests use /api prefix which is either:
// - Proxied by Next.js rewrites in development (strips /api, forwards to backend)
// - Handled by reverse proxy in production (strips /api, forwards to backend)
const API_BASE = "/api";

export class ApiError extends Error {
  constructor(
    public status: number,
    public statusText: string,
    message?: string
  ) {
    super(message || `API Error: ${status} ${statusText}`);
    this.name = "ApiError";
  }
}

async function request<T>(
  endpoint: string,
  options: RequestInit = {}
): Promise<{ data: T }> {
  const response = await fetch(`${API_BASE}${endpoint}`, {
    ...options,
    credentials: "include", // Include cookies for auth
    headers: {
      "Content-Type": "application/json",
      ...options.headers,
    },
  });

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

// Legacy export for backwards compatibility
export async function apiClient<T>(
  endpoint: string,
  options: RequestInit = {}
): Promise<T> {
  const result = await request<T>(endpoint, options);
  return result.data;
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
  if (typeof window !== 'undefined') {
    return window.location.origin + API_BASE;
  }
  return API_BASE;
}
