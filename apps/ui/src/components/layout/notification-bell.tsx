"use client";

import { useRouter } from "next/navigation";
import { useHealthIssues } from "@/hooks/use-health-issues";
import { Bell, AlertTriangle } from "lucide-react";
import {
  useNotificationsContext,
  useOptionalNotificationsContext,
} from "@/providers/notifications-provider";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

function formatNotificationTime(timestamp: string): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
    month: "short",
    day: "numeric",
  }).format(new Date(timestamp));
}

function NotificationMenuContent() {
  const router = useRouter();
  const { notifications, isEnabled, openNotification, markViewed } = useNotificationsContext();
  const health = useHealthIssues(undefined, 0, isEnabled);
  const activity = notifications.filter(
    (n) => n.kind !== "health.issue" || n.payload?.status === "resolved",
  );

  if (!isEnabled) {
    return null;
  }

  return (
    <>
      <DropdownMenuGroup>
        <DropdownMenuLabel className="flex items-center justify-between">
          Action required
          <span className="text-xs text-muted-foreground">
            {health.data?.total ?? "…"} unresolved
          </span>
        </DropdownMenuLabel>
        {health.isError && (
          <div className="px-3 py-2 text-xs text-muted-foreground">
            Could not load integration health.
          </div>
        )}
        {health.data?.total === 0 && (
          <div className="px-3 py-2 text-xs text-muted-foreground">No pending issues detected</div>
        )}
        {(health.data?.data ?? []).slice(0, 5).map((issue) => (
          <DropdownMenuItem
            key={issue.id}
            className="block px-3 py-3"
            onClick={() => {
              if (issue.notification_id) void markViewed(issue.notification_id);
              router.push(issue.href);
            }}
          >
            <div className="flex gap-2">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
              <div className="min-w-0">
                <p className="text-sm font-medium">{issue.title}</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {issue.agent_name} · {issue.stale ? "Needs check" : "Action required"}
                </p>
                <p className="mt-1 text-xs text-muted-foreground">{issue.body}</p>
                <p className="mt-2 text-xs underline">Review issue</p>
              </div>
            </div>
          </DropdownMenuItem>
        ))}
        <DropdownMenuItem onClick={() => router.push("/settings/health")}>
          View all health issues
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Activity</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {activity.length === 0 ? (
          <div className="px-3 py-6 text-sm text-muted-foreground">No notifications</div>
        ) : (
          activity.map((notification) => (
            <DropdownMenuItem
              key={notification.id}
              className="block px-3 py-3"
              onClick={() => openNotification(notification)}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <p
                    className={cn(
                      "truncate text-sm",
                      notification.viewed_at ? "text-muted-foreground" : "font-medium",
                    )}
                  >
                    {notification.title}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">{notification.body}</p>
                  <p className="mt-2 text-[11px] text-muted-foreground">
                    {formatNotificationTime(notification.created_at)}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  {notification.occurrence_count > 1 && (
                    <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">
                      x{notification.occurrence_count}
                    </span>
                  )}
                  {!notification.viewed_at && (
                    <button
                      type="button"
                      className="rounded px-2 py-1 text-[10px] text-muted-foreground hover:bg-muted hover:text-foreground"
                      onClick={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        void markViewed(notification.id);
                      }}
                    >
                      View
                    </button>
                  )}
                </div>
              </div>
            </DropdownMenuItem>
          ))
        )}
      </DropdownMenuGroup>
      {notifications.some(
        (notification) => notification.target_type === "session" && notification.href,
      ) && (
        <>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => router.push("/sessions")}>
            Open sessions
          </DropdownMenuItem>
        </>
      )}
    </>
  );
}

export function NotificationBell() {
  const { unviewedCount, isEnabled } = useNotificationsContext();
  const health = useHealthIssues(undefined, 0, isEnabled);
  const hasIssues = (health.data?.total ?? 0) > 0;

  if (!isEnabled) {
    return null;
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(
          "relative inline-flex h-9 w-9 items-center justify-center border border-transparent transition-colors hover:border-border hover:bg-background",
          unviewedCount > 0 ? "text-foreground" : "text-muted-foreground",
        )}
        aria-label={`Notifications, ${unviewedCount} unread, ${health.data?.total ?? 0} unresolved issues`}
      >
        <Bell className="h-4 w-4" />
        {hasIssues && (
          <span className="absolute bottom-0 right-0 size-2 bg-warning" aria-hidden="true" />
        )}
        {unviewedCount > 0 && (
          <span className="absolute -right-1 -top-1 min-w-5 rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-medium leading-none text-primary-foreground">
            {unviewedCount > 9 ? "9+" : unviewedCount}
          </span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuPositioner side="bottom" align="end">
        <DropdownMenuContent className="w-[22rem] max-w-[calc(100vw-1rem)]">
          <NotificationMenuContent />
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}

export function NotificationIndicator() {
  const context = useOptionalNotificationsContext();
  const health = useHealthIssues(undefined, 0, !!context?.isEnabled);
  const hasIssues = (health.data?.total ?? 0) > 0;

  if (!context?.isEnabled || (context.unviewedCount === 0 && !hasIssues)) {
    return null;
  }

  return (
    <>
      <span
        aria-hidden="true"
        className={
          hasIssues
            ? "h-2.5 w-2.5 shrink-0 rounded-full bg-warning"
            : "h-2.5 w-2.5 shrink-0 rounded-full bg-primary"
        }
      />
      <span className="sr-only">
        {context.unviewedCount} unread notifications; {health.data?.total ?? 0} unresolved issues
      </span>
    </>
  );
}

export function NotificationMenuSub() {
  const context = useOptionalNotificationsContext();
  const health = useHealthIssues(undefined, 0, !!context?.isEnabled);
  const hasIssues = (health.data?.total ?? 0) > 0;

  if (!context?.isEnabled) {
    return null;
  }

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <Bell className="icon-sharp mr-2 h-4 w-4" />
        Notifications
        {hasIssues && <AlertTriangle className="ml-1 h-3 w-3 text-warning" />}
        {context.unviewedCount > 0 && (
          <span className="ml-auto rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-medium leading-none text-primary-foreground">
            {context.unviewedCount > 9 ? "9+" : context.unviewedCount}
          </span>
        )}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="w-[22rem] max-w-[calc(100vw-1rem)]">
        <NotificationMenuContent />
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}
