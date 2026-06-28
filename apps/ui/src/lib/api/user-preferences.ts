import { api, ApiError } from "./client";

// Per-user key/value store for client/UI settings (e.g. dismissed banners).
// Values are arbitrary JSON, persisted server-side so preferences follow the
// user across devices and browsers.
export interface UserPreference {
  key: string;
  value: unknown;
  updated_at: string;
}

export async function listUserPreferences(): Promise<UserPreference[]> {
  const response = await api.get<UserPreference[]>("/v1/user/preferences");
  return response.data;
}

/**
 * Read a single preference by key. Returns null when the preference has not
 * been set yet (the API responds 404 in that case).
 */
export async function getUserPreference(key: string): Promise<UserPreference | null> {
  try {
    const response = await api.get<UserPreference>(
      `/v1/user/preferences/${encodeURIComponent(key)}`,
    );
    return response.data;
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function setUserPreference(key: string, value: unknown): Promise<UserPreference> {
  const response = await api.put<UserPreference>(
    `/v1/user/preferences/${encodeURIComponent(key)}`,
    { value },
  );
  return response.data;
}
