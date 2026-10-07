// Durable user notifications (`/v1/notifications`).

/** What sent a notification: an agent, or the system itself. */
export interface NotificationSource {
  type: string;
  id?: string | null;
  name?: string | null;
}

/** Durable user notification */
export interface Notification {
  id: string;
  kind: string;
  title: string;
  body: string;
  source?: NotificationSource | null;
  target_type?: string | null;
  target_id?: string | null;
  href?: string | null;
  payload: Record<string, unknown>;
  occurrence_count: number;
  viewed_at?: string | null;
  created_at: string;
  updated_at: string;
}

/** Notification list response with an accurate bell counter */
export interface ListNotificationsResponse {
  data: Notification[];
  unviewed_count: number;
}
