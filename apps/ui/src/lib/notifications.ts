import type { Notification } from "@/lib/api/types";

const SESSION_CHAT_PATH_REGEX = /^\/sessions\/([^/]+)\/chat(?:\/|$)/;

export function getNotificationTargetKey(notification: Notification): string | null {
  if (!notification.target_type || !notification.target_id) {
    return null;
  }
  return `${notification.target_type}:${notification.target_id}`;
}

export function getActiveNotificationTarget(pathname: string): string | null {
  const match = pathname.match(SESSION_CHAT_PATH_REGEX);
  if (!match) {
    return null;
  }
  return `session:${match[1]}`;
}

export function shouldSuppressNotification(
  notification: Notification,
  activeTargetKey: string | null,
  isVisible: boolean,
  isFocused: boolean,
): boolean {
  if (!activeTargetKey || !isVisible || !isFocused || notification.viewed_at) {
    return false;
  }
  return getNotificationTargetKey(notification) === activeTargetKey;
}
