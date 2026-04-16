/**
 * Decisions:
 * - User footer owns auth-only actions; anonymous mode stays a simple version label.
 * - Route pushes happen inside menu actions so parent layout does not coordinate menu details.
 * - Forks can append profile menu items via a render prop instead of replacing this component.
 */
"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { ChevronUp, Key, LogOut, User } from "lucide-react";
import { McpConnectDialog, McpConnectMenuItem } from "@/components/layout/mcp-connect-button";
import { NotificationIndicator, NotificationMenuSub } from "@/components/layout/notification-bell";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export type SidebarUserMenuUser = {
  name?: string | null;
  email: string;
  avatar_url?: string | null;
};

export type SidebarUserMenuItemsRenderer = (context: {
  user: SidebarUserMenuUser;
  navigate: (href: string) => void;
}) => React.ReactNode;

function getInitials(name: string): string {
  if (!name.trim()) return "";
  return name
    .split(" ")
    .filter((part) => part.length > 0)
    .map((part) => part[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
}

export function SidebarUserMenu({
  requiresAuth,
  user,
  logout,
  logoutPending,
  renderExtraItems,
  version,
}: {
  requiresAuth: boolean;
  user: SidebarUserMenuUser | null;
  logout: () => Promise<void>;
  logoutPending: boolean;
  renderExtraItems?: SidebarUserMenuItemsRenderer;
  version: string;
}) {
  const router = useRouter();
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false);
  const navigate = (href: string) => router.push(href);

  const handleLogout = async () => {
    await logout();
    navigate("/login");
  };

  if (!requiresAuth || !user) {
    return <p className="px-2.5 text-[11px] text-muted-foreground">Everruns v{version}</p>;
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger className="flex w-full items-center gap-2.5 border border-transparent px-2.5 py-1.5 text-[13px] transition-colors hover:border-border hover:bg-card">
          <Avatar className="h-7 w-7">
            {user.avatar_url && <AvatarImage src={user.avatar_url} alt={user.name || user.email} />}
            <AvatarFallback>
              {user.name ? getInitials(user.name) : <User className="h-4 w-4" />}
            </AvatarFallback>
          </Avatar>
          <div className="flex-1 text-left">
            <p className="truncate font-semibold leading-5">{user.name || user.email}</p>
            {user.name && (
              <p className="truncate text-[11px] text-muted-foreground">{user.email}</p>
            )}
          </div>
          <NotificationIndicator />
          <ChevronUp className="icon-sharp h-4 w-4 text-muted-foreground" />
        </DropdownMenuTrigger>
        <DropdownMenuPositioner side="top" align="start">
          <DropdownMenuContent className="w-56">
            <DropdownMenuGroup>
              <DropdownMenuLabel>My Account</DropdownMenuLabel>
              <NotificationMenuSub />
              <DropdownMenuItem onClick={() => navigate("/settings")}>
                <User className="icon-sharp mr-2 h-4 w-4" />
                Profile
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => navigate("/settings/api-keys")}>
                <Key className="icon-sharp mr-2 h-4 w-4" />
                API Keys
              </DropdownMenuItem>
              <McpConnectMenuItem onSelect={() => setMcpDialogOpen(true)} />
              {renderExtraItems?.({ user, navigate })}
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onClick={handleLogout} disabled={logoutPending}>
              <LogOut className="icon-sharp mr-2 h-4 w-4" />
              {logoutPending ? "Signing out..." : "Sign out"}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenuPositioner>
      </DropdownMenu>
      <McpConnectDialog open={mcpDialogOpen} onOpenChange={setMcpDialogOpen} />
    </>
  );
}
