"use client";

// Important decisions:
// - Keep raw notifications in provider state, then derive suppressed/effective UI state
//   before rendering bell count or toasts. This avoids flicker for active-chat suppression.
// - Treat the server as the durable source of truth, but apply active-target auto-view on the
//   client for V1. Matching notifications never enter rendered state while the chat is visible.
// - Toasts are a secondary delivery surface layered on top of the same canonical notifications.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { usePathname, useRouter } from "next/navigation";
import { X } from "lucide-react";
import { useMountEffect } from "@/hooks/use-mount-effect";
import { useAuth } from "@/providers/auth-provider";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useNotifications } from "@/hooks/use-notifications";
import { markNotificationViewed, getNotificationsSseUrl } from "@/lib/api/notifications";
import { createReconnectTracker } from "@/lib/sse-reconnect";
import {
  getActiveNotificationTarget,
  getNotificationTargetKey,
  shouldSuppressNotification,
} from "@/lib/notifications";
import type { Notification } from "@/lib/api/types";

type NotificationState = {
  rawNotifications: Notification[];
  rawUnviewedCount: number;
  initialized: boolean;
};

type ToastNotification = Notification & { toast_id: string };

interface NotificationsContextValue {
  notifications: Notification[];
  unviewedCount: number;
  isEnabled: boolean;
  markViewed: (notificationId: string) => Promise<void>;
  openNotification: (notification: Notification) => void;
}

const NotificationsContext = createContext<NotificationsContextValue | undefined>(undefined);

function sortNotifications(notifications: Notification[]): Notification[] {
  return [...notifications].sort((a, b) => {
    const createdDiff = new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
    if (createdDiff !== 0) return createdDiff;
    return b.id.localeCompare(a.id);
  });
}

function upsertNotification(state: NotificationState, incoming: Notification): NotificationState {
  const previous = state.rawNotifications.find((item) => item.id === incoming.id);
  const notifications = sortNotifications(
    previous
      ? state.rawNotifications.map((item) => (item.id === incoming.id ? incoming : item))
      : [incoming, ...state.rawNotifications],
  ).slice(0, 50);

  let rawUnviewedCount = state.rawUnviewedCount;
  if (!previous && !incoming.viewed_at) {
    rawUnviewedCount += 1;
  } else if (previous && !previous.viewed_at && incoming.viewed_at) {
    rawUnviewedCount = Math.max(0, rawUnviewedCount - 1);
  } else if (previous && previous.viewed_at && !incoming.viewed_at) {
    rawUnviewedCount += 1;
  }

  return {
    rawNotifications: notifications,
    rawUnviewedCount,
    initialized: state.initialized,
  };
}

function updateViewedInState(state: NotificationState, notificationId: string): NotificationState {
  const existing = state.rawNotifications.find((item) => item.id === notificationId);
  if (!existing || existing.viewed_at) {
    return state;
  }
  return upsertNotification(state, {
    ...existing,
    viewed_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  });
}

export function NotificationsProvider({ children }: { children: ReactNode }) {
  const router = useRouter();
  const pathname = usePathname();
  const { requiresAuth, user } = useAuth();
  const notificationsFeatureEnabled = useFeatureFlag("notifications");
  const isEnabled = notificationsFeatureEnabled && requiresAuth && !!user;
  const notificationsQuery = useNotifications(isEnabled);

  const [state, setState] = useState<NotificationState>({
    rawNotifications: [],
    rawUnviewedCount: 0,
    initialized: false,
  });
  const [toastQueue, setToastQueue] = useState<ToastNotification[]>([]);
  const [isVisible, setIsVisible] = useState(true);
  const [isFocused, setIsFocused] = useState(true);
  const reconnectRef = useRef(createReconnectTracker());
  const lastUpdatedAtRef = useRef<string | undefined>(undefined);
  const eventSourceRef = useRef<EventSource | null>(null);
  const autoViewingIdsRef = useRef<Set<string>>(new Set());
  const toastIdsRef = useRef<Set<string>>(new Set());
  const activeTargetKey = useMemo(() => getActiveNotificationTarget(pathname), [pathname]);
  const activeTargetKeyRef = useRef<string | null>(activeTargetKey);
  const isVisibleRef = useRef(isVisible);
  const isFocusedRef = useRef(isFocused);

  useEffect(() => {
    if (!isEnabled) {
      setState({ rawNotifications: [], rawUnviewedCount: 0, initialized: false });
      setToastQueue([]);
      toastIdsRef.current.clear();
      autoViewingIdsRef.current.clear();
      eventSourceRef.current?.close();
      eventSourceRef.current = null;
    }
  }, [isEnabled]);

  useEffect(() => {
    if (!notificationsQuery.data || !isEnabled) return;
    setState({
      rawNotifications: sortNotifications(notificationsQuery.data.data).slice(0, 50),
      rawUnviewedCount: notificationsQuery.data.unviewed_count,
      initialized: true,
    });
  }, [notificationsQuery.data, isEnabled]);

  // Derive latest updated_at inline — avoids an extra render cycle via useEffect.
  lastUpdatedAtRef.current = state.rawNotifications.reduce<string | undefined>(
    (latest, current) => {
      if (!latest) return current.updated_at;
      return new Date(current.updated_at) > new Date(latest) ? current.updated_at : latest;
    },
    undefined,
  );

  useMountEffect(() => {
    if (typeof document === "undefined") return;

    const updateVisibility = () => setIsVisible(document.visibilityState === "visible");
    const onFocus = () => setIsFocused(true);
    const onBlur = () => setIsFocused(false);

    updateVisibility();
    setIsFocused(document.hasFocus());
    document.addEventListener("visibilitychange", updateVisibility);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    return () => {
      document.removeEventListener("visibilitychange", updateVisibility);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
    };
  });

  // Keep refs in sync — assigned inline during render so SSE callbacks
  // always read the latest values without requiring effect round-trips.
  activeTargetKeyRef.current = activeTargetKey;
  isVisibleRef.current = isVisible;
  isFocusedRef.current = isFocused;

  const enqueueToast = useCallback((notification: Notification) => {
    if (toastIdsRef.current.has(notification.id)) return;
    toastIdsRef.current.add(notification.id);
    const toast_id = `${notification.id}:${Date.now()}`;
    setToastQueue((prev) => [...prev, { ...notification, toast_id }]);
    window.setTimeout(() => {
      setToastQueue((prev) => prev.filter((item) => item.toast_id !== toast_id));
    }, 5000);
  }, []);

  const markViewed = useCallback(async (notificationId: string) => {
    setState((prev) => updateViewedInState(prev, notificationId));
    try {
      const updated = await markNotificationViewed(notificationId);
      setState((prev) => upsertNotification(prev, updated));
    } finally {
      autoViewingIdsRef.current.delete(notificationId);
    }
  }, []);

  const openNotification = useCallback(
    (notification: Notification) => {
      if (!notification.viewed_at) {
        void markViewed(notification.id);
      }
      if (notification.href) {
        router.push(notification.href);
      }
    },
    [markViewed, router],
  );

  useEffect(() => {
    if (!isEnabled || !state.initialized) return;
    let cancelled = false;

    const connect = () => {
      if (cancelled) return;
      eventSourceRef.current?.close();
      const eventSource = new EventSource(getNotificationsSseUrl(lastUpdatedAtRef.current), {
        withCredentials: true,
      });
      eventSourceRef.current = eventSource;

      eventSource.addEventListener("connected", () => {
        reconnectRef.current.reset();
      });

      eventSource.addEventListener("disconnecting", (messageEvent) => {
        try {
          const data = JSON.parse(messageEvent.data) as { retry_ms?: number };
          eventSource.close();
          const retryMs = reconnectRef.current.onGraceful(data.retry_ms ?? 100);
          window.setTimeout(() => {
            if (!cancelled) connect();
          }, retryMs);
        } catch {
          eventSource.close();
          if (!cancelled) connect();
        }
      });

      eventSource.addEventListener("notification.upsert", (messageEvent) => {
        try {
          const incoming = JSON.parse(messageEvent.data) as Notification;
          const suppressed = shouldSuppressNotification(
            incoming,
            activeTargetKeyRef.current,
            isVisibleRef.current,
            isFocusedRef.current,
          );

          let shouldToast = false;
          setState((prev) => {
            const next = upsertNotification(prev, incoming);
            const existed = prev.rawNotifications.some((item) => item.id === incoming.id);
            shouldToast =
              !existed &&
              !incoming.viewed_at &&
              !suppressed &&
              isVisibleRef.current &&
              isFocusedRef.current &&
              next.initialized;
            return next;
          });

          if (suppressed && !incoming.viewed_at && !autoViewingIdsRef.current.has(incoming.id)) {
            autoViewingIdsRef.current.add(incoming.id);
            void markViewed(incoming.id);
          } else if (shouldToast) {
            enqueueToast(incoming);
          }
        } catch (error) {
          console.error("Failed to parse notification SSE event:", error);
        }
      });

      eventSource.onerror = () => {
        eventSource.close();
        const retryMs = reconnectRef.current.onError();
        if (retryMs !== null) {
          window.setTimeout(() => {
            if (!cancelled) connect();
          }, retryMs);
        }
      };
    };

    connect();
    return () => {
      cancelled = true;
      eventSourceRef.current?.close();
    };
  }, [enqueueToast, isEnabled, markViewed, state.initialized]);

  const suppressedUnreadIds = useMemo(
    () =>
      state.rawNotifications
        .filter((notification) =>
          shouldSuppressNotification(notification, activeTargetKey, isVisible, isFocused),
        )
        .map((notification) => notification.id),
    [activeTargetKey, isFocused, isVisible, state.rawNotifications],
  );

  useEffect(() => {
    if (!suppressedUnreadIds.length) return;
    for (const notificationId of suppressedUnreadIds) {
      if (autoViewingIdsRef.current.has(notificationId)) continue;
      autoViewingIdsRef.current.add(notificationId);
      void markViewed(notificationId);
    }
  }, [markViewed, suppressedUnreadIds]);

  const suppressedUnreadSet = useMemo(() => new Set(suppressedUnreadIds), [suppressedUnreadIds]);
  const notifications = useMemo(
    () =>
      state.rawNotifications.filter(
        (notification) =>
          !(
            !notification.viewed_at &&
            suppressedUnreadSet.has(notification.id) &&
            getNotificationTargetKey(notification) === activeTargetKey
          ),
      ),
    [activeTargetKey, state.rawNotifications, suppressedUnreadSet],
  );

  const unviewedCount = Math.max(0, state.rawUnviewedCount - suppressedUnreadIds.length);

  const contextValue = useMemo<NotificationsContextValue>(
    () => ({
      notifications,
      unviewedCount,
      isEnabled,
      markViewed,
      openNotification,
    }),
    [isEnabled, markViewed, notifications, openNotification, unviewedCount],
  );

  return (
    <NotificationsContext.Provider value={contextValue}>
      {children}
      {toastQueue.length > 0 && (
        <div className="pointer-events-none fixed right-4 top-4 z-50 flex w-80 flex-col gap-2">
          {toastQueue.map((toast) => (
            <div
              key={toast.toast_id}
              className="pointer-events-auto border bg-card/95 p-3 shadow-lg backdrop-blur"
            >
              <div className="flex items-start gap-3">
                <button
                  type="button"
                  className="min-w-0 flex-1 text-left"
                  onClick={() => openNotification(toast)}
                >
                  <p className="text-sm font-medium">{toast.title}</p>
                  <p className="mt-1 text-sm text-muted-foreground">{toast.body}</p>
                </button>
                <button
                  type="button"
                  className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
                  onClick={() =>
                    setToastQueue((prev) => prev.filter((item) => item.toast_id !== toast.toast_id))
                  }
                  aria-label="Dismiss notification"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </NotificationsContext.Provider>
  );
}

export function useNotificationsContext() {
  const context = useContext(NotificationsContext);
  if (!context) {
    throw new Error("useNotificationsContext must be used within a NotificationsProvider");
  }
  return context;
}
