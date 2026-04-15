"use client";

import { useRouter } from "next/navigation";
import { Bell } from "lucide-react";
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

  if (!isEnabled) {
    return null;
  }

  return (
    <>
      <DropdownMenuGroup>
        <DropdownMenuLabel>Notifications</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {notifications.length === 0 ? (
          <div className="px-3 py-6 text-sm text-muted-foreground">No notifications</div>
        ) : (
          notifications.map((notification) => (
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
      {notifications.some((notification) => notification.href) && (
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
        aria-label="Notifications"
      >
        <Bell className="h-4 w-4" />
        {unviewedCount > 0 && (
          <span className="absolute -right-1 -top-1 min-w-5 rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-medium leading-none text-primary-foreground">
            {unviewedCount > 9 ? "9+" : unviewedCount}
          </span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuPositioner side="bottom" align="end">
        <DropdownMenuContent className="w-[22rem]">
          <NotificationMenuContent />
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}

export function NotificationIndicator() {
  const context = useOptionalNotificationsContext();

  if (!context?.isEnabled || context.unviewedCount === 0) {
    return null;
  }

  return <span aria-label="Unread notifications" className="h-2.5 w-2.5 rounded-full bg-primary" />;
}

export function NotificationMenuSub() {
  const context = useOptionalNotificationsContext();

  if (!context?.isEnabled) {
    return null;
  }

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <Bell className="icon-sharp mr-2 h-4 w-4" />
        Notifications
        {context.unviewedCount > 0 && (
          <span className="ml-auto rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-medium leading-none text-primary-foreground">
            {context.unviewedCount > 9 ? "9+" : context.unviewedCount}
          </span>
        )}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="w-[22rem]">
        <NotificationMenuContent />
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}
