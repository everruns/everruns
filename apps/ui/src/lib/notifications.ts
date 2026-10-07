import type { Notification } from "@/lib/api/types";

// A chat is open either in Chat (`/chats/{id}`) or in the session inspector's
// transcript (`/sessions/{id}/chat`); both count as "looking at it".
const CHAT_PATH_REGEXES = [
  /^\/chats\/(session_[^/]+)(?:\/|$)/,
  /^\/sessions\/([^/]+)\/chat(?:\/|$)/,
];

export function getNotificationTargetKey(notification: Notification): string | null {
  if (!notification.target_type || !notification.target_id) {
    return null;
  }
  return `${notification.target_type}:${notification.target_id}`;
}

export function getActiveNotificationTarget(pathname: string): string | null {
  for (const regex of CHAT_PATH_REGEXES) {
    const match = pathname.match(regex);
    if (match) {
      return `session:${match[1]}`;
    }
  }
  return null;
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

function formatNotificationTime(timestamp: string): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
    month: "short",
    day: "numeric",
  }).format(new Date(timestamp));
}

/** "Sender · time" line under a notification; the sender is omitted when unknown. */
export function notificationMeta(notification: Notification): string {
  const time = formatNotificationTime(notification.created_at);
  const sender = notification.source?.name?.trim();
  return sender ? `${sender} · ${time}` : time;
}
