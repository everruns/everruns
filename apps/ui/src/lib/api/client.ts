// API base URL configuration:
// - In production: defaults to "/api" (same-origin, requires reverse proxy)
// - In development: defaults to "http://localhost:9000"
// - Can be overridden via NEXT_PUBLIC_API_BASE_URL environment variable
const API_BASE =
  process.env.NEXT_PUBLIC_API_BASE_URL ||
  (process.env.NODE_ENV === "production" ? "/api" : "http://localhost:9000");

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

export async function apiClient<T>(
  endpoint: string,
  options: RequestInit = {}
): Promise<T> {
  const response = await fetch(`${API_BASE}${endpoint}`, {
    ...options,
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

  // Handle empty responses (204 No Content)
  if (response.status === 204) {
    return {} as T;
  }

  return response.json();
}

export function getApiBaseUrl(): string {
  return API_BASE;
}
