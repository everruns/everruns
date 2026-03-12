import { api, getApiBaseUrl } from "./client";
import type { ListNotificationsResponse, Notification } from "./types";

export async function listNotifications(limit: number = 50): Promise<ListNotificationsResponse> {
  const response = await api.get<ListNotificationsResponse>(`/v1/notifications?limit=${limit}`);
  return response.data;
}

export async function markNotificationViewed(notificationId: string): Promise<Notification> {
  const response = await api.post<Notification>(`/v1/notifications/${notificationId}/view`);
  return response.data;
}

export function getNotificationsSseUrl(since?: string): string {
  const baseUrl = getApiBaseUrl();
  const params = new URLSearchParams();
  if (since) {
    params.set("since", since);
  }
  const queryString = params.toString();
  return `${baseUrl}/v1/notifications/sse${queryString ? `?${queryString}` : ""}`;
}
