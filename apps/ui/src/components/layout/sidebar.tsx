"use client";

import { useState } from "react";
import { version } from "../../../package.json";
import Link from "next/link";
import Image from "next/image";
import { usePathname, useRouter } from "next/navigation";
import { cn } from "@/lib/utils";
import { useAuth } from "@/providers/auth-provider";
import { useFeatureFlags } from "@/providers/feature-flags-provider";
import { useOrg } from "@/providers/org-provider";
import { useLogout } from "@/hooks/use-auth";
import { useCreateOrganization } from "@/hooks/use-organizations";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuPositioner,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuLabel,
} from "@/components/ui/dropdown-menu";
import {
  LayoutDashboard,
  Boxes,
  MessageSquare,
  Puzzle,
  Settings,
  LogOut,
  User,
  Key,
  ChevronUp,
  ChevronDown,
  FlaskConical,
  Calendar,
  MessageCircle,
  Cog,
  Server,
  Workflow,
  Building2,
  Check,
  Shield,
  BookOpen,
  Plus,
  Rocket,
} from "lucide-react";

const isDev = process.env.NODE_ENV === "development";

type NavItem = {
  name: string;
  href: string;
  icon: React.ComponentType<{ className?: string }>;
  flag?: keyof import("@/lib/api/types").FeatureFlags;
  exact?: boolean;
};

const topNavigation: NavItem[] = [
  { name: "Chat", href: "/chat", icon: MessageCircle, flag: "global_chat" },
  { name: "Dashboard", href: "/dashboard", icon: LayoutDashboard },
  { name: "Sessions", href: "/sessions", icon: MessageSquare },
];

const buildingBlocksNavigation = [
  { name: "Harnesses", href: "/harnesses", icon: Shield },
  { name: "Agents", href: "/agents", icon: Boxes },
  { name: "Skills", href: "/skills", icon: BookOpen },
  { name: "Capabilities", href: "/capabilities", icon: Puzzle },
  { name: "Apps", href: "/apps", icon: Rocket },
];

const bottomNavigation = [{ name: "Settings", href: "/settings", icon: Settings }];

const durableNavigation = [
  { name: "Overview", href: "/durable", icon: Cog, exact: true },
  { name: "Workers", href: "/durable/workers", icon: Server },
  { name: "Workflows", href: "/durable/workflows", icon: Workflow },
  { name: "Schedules", href: "/durable/schedules", icon: Calendar },
];

const devNavigation = [{ name: "Dev Tools", href: "/dev", icon: FlaskConical }];

function getInitials(name: string): string {
  if (!name.trim()) return "";
  return name
    .split(" ")
    .filter((n) => n.length > 0)
    .map((n) => n[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
}

function CreateOrganizationDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const createOrg = useCreateOrganization();
  const { setCurrentOrg } = useOrg();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    const org = await createOrg.mutateAsync({ name: name.trim() });
    // Switch to the newly created org
    setCurrentOrg({ public_id: org.id, name: org.name, role: "owner" });
    setName("");
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create Organisation</DialogTitle>
          <DialogDescription>
            Create a new organisation. You will be added as a member automatically.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="org-name">Name</Label>
            <Input
              id="org-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Organisation name"
              required
            />
          </div>
          {createOrg.isError && (
            <p className="text-sm text-destructive">
              Failed to create organisation: {createOrg.error.message}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createOrg.isPending || !name.trim()}>
              {createOrg.isPending ? "Creating..." : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function Sidebar() {
  const pathname = usePathname();
  const router = useRouter();
  const { user, requiresAuth } = useAuth();
  const featureFlags = useFeatureFlags();
  const { currentOrg, organizations, setCurrentOrg } = useOrg();
  const logoutMutation = useLogout();
  const [createOrgOpen, setCreateOrgOpen] = useState(false);

  const handleLogout = async () => {
    await logoutMutation.mutateAsync();
    router.push("/login");
  };

  return (
    <div className="flex h-full w-64 flex-col border-r bg-card">
      {/* Logo */}
      <div className="flex h-16 items-center border-b px-6">
        <Link href="/dashboard" className="flex items-center gap-2">
          <Image src="/logo.svg" alt="Everruns" width={32} height={32} />
          <span className="text-xl font-bold">Everruns</span>
        </Link>
      </div>

      {/* Organization Selector */}
      {organizations.length > 0 && (
        <div className="border-b px-3 py-2">
          <DropdownMenu>
            <DropdownMenuTrigger className="flex w-full items-center gap-2 px-3 py-2 text-sm hover:bg-muted transition-colors">
              <Building2 className="h-4 w-4 text-muted-foreground" />
              <span className="flex-1 text-left truncate font-medium">
                {currentOrg?.name ?? "Select Organization"}
              </span>
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            </DropdownMenuTrigger>
            <DropdownMenuPositioner side="bottom" align="start">
              <DropdownMenuContent className="w-56">
                <DropdownMenuGroup>
                  <DropdownMenuLabel>Organizations</DropdownMenuLabel>
                  {organizations.map((org) => (
                    <DropdownMenuItem
                      key={org.public_id}
                      onClick={() => setCurrentOrg(org)}
                      className="flex items-center justify-between"
                    >
                      <span className="truncate">{org.name}</span>
                      {currentOrg?.public_id === org.public_id && (
                        <Check className="h-4 w-4 text-primary" />
                      )}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuGroup>
                <DropdownMenuSeparator />
                <DropdownMenuItem onClick={() => setCreateOrgOpen(true)}>
                  <Plus className="mr-2 h-4 w-4" />
                  Create Organisation
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        </div>
      )}

      {/* Navigation */}
      <nav className="flex-1 min-h-0 overflow-y-auto space-y-1 py-4">
        {topNavigation
          .filter((item) => !item.flag || featureFlags[item.flag])
          .map((item) => {
            const isActive = pathname === item.href || pathname.startsWith(`${item.href}/`);
            return (
              <Link
                key={item.name}
                href={item.href}
                className={cn(
                  "flex items-center gap-3 px-3 py-2 text-sm font-medium transition-colors",
                  isActive
                    ? "bg-accent/10 text-accent-foreground border-l-2 border-accent"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground border-l-2 border-transparent",
                )}
              >
                <item.icon className="h-5 w-5" />
                {item.name}
              </Link>
            );
          })}

        {/* Building Blocks section */}
        <div className="my-3 border-t" />
        <p className="px-3 py-1 text-xs font-medium text-muted-foreground">Building Blocks</p>
        {buildingBlocksNavigation.map((item) => {
          const isActive = pathname === item.href || pathname.startsWith(`${item.href}/`);
          return (
            <Link
              key={item.name}
              href={item.href}
              className={cn(
                "flex items-center gap-3 px-3 py-2 text-sm font-medium transition-colors",
                isActive
                  ? "bg-accent/10 text-accent-foreground border-l-2 border-accent"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground border-l-2 border-transparent",
              )}
            >
              <item.icon className="h-5 w-5" />
              {item.name}
            </Link>
          );
        })}

        {/* Settings */}
        <div className="my-3 border-t" />
        {bottomNavigation.map((item) => {
          const isActive = pathname === item.href || pathname.startsWith(`${item.href}/`);
          return (
            <Link
              key={item.name}
              href={item.href}
              className={cn(
                "flex items-center gap-3 px-3 py-2 text-sm font-medium transition-colors",
                isActive
                  ? "bg-accent/10 text-accent-foreground border-l-2 border-accent"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground border-l-2 border-transparent",
              )}
            >
              <item.icon className="h-5 w-5" />
              {item.name}
            </Link>
          );
        })}

        {/* Durable Execution section */}
        <div className="my-3 border-t" />
        <p className="px-3 py-1 text-xs font-medium text-muted-foreground">Durable Execution</p>
        {durableNavigation.map((item) => {
          const isActive = item.exact
            ? pathname === item.href
            : pathname === item.href || pathname.startsWith(`${item.href}/`);
          return (
            <Link
              key={item.name}
              href={item.href}
              className={cn(
                "flex items-center gap-3 px-3 py-2 text-sm font-medium transition-colors",
                isActive
                  ? "bg-accent/10 text-accent-foreground border-l-2 border-accent"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground border-l-2 border-transparent",
              )}
            >
              <item.icon className="h-5 w-5" />
              {item.name}
            </Link>
          );
        })}

        {/* Dev-only navigation */}
        {isDev && (
          <>
            <div className="my-3 border-t" />
            <p className="px-3 py-1 text-xs font-medium text-muted-foreground">Dev</p>
            {devNavigation.map((item) => {
              const isActive = pathname === item.href || pathname.startsWith(`${item.href}/`);
              return (
                <Link
                  key={item.name}
                  href={item.href}
                  className={cn(
                    "flex items-center gap-3 px-3 py-2 text-sm font-medium transition-colors",
                    isActive
                      ? "bg-accent/10 text-accent-foreground border-l-2 border-accent"
                      : "text-muted-foreground hover:bg-muted hover:text-foreground",
                  )}
                >
                  <item.icon className="h-5 w-5" />
                  {item.name}
                </Link>
              );
            })}
          </>
        )}
      </nav>

      {/* User menu / Footer */}
      <div className="border-t p-3">
        {requiresAuth && user ? (
          <DropdownMenu>
            <DropdownMenuTrigger className="flex w-full items-center gap-3 px-3 py-2 text-sm hover:bg-muted transition-colors">
              <Avatar className="h-8 w-8">
                {user.avatar_url && (
                  <AvatarImage src={user.avatar_url} alt={user.name || user.email} />
                )}
                <AvatarFallback>
                  {user.name ? getInitials(user.name) : <User className="h-4 w-4" />}
                </AvatarFallback>
              </Avatar>
              <div className="flex-1 text-left">
                <p className="font-medium truncate">{user.name || user.email}</p>
                {user.name && (
                  <p className="text-xs text-muted-foreground truncate">{user.email}</p>
                )}
              </div>
              <ChevronUp className="h-4 w-4 text-muted-foreground" />
            </DropdownMenuTrigger>
            <DropdownMenuPositioner side="top" align="start">
              <DropdownMenuContent className="w-56">
                <DropdownMenuGroup>
                  <DropdownMenuLabel>My Account</DropdownMenuLabel>
                  <DropdownMenuItem onClick={() => router.push("/settings")}>
                    <User className="mr-2 h-4 w-4" />
                    Profile
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => router.push("/settings/api-keys")}>
                    <Key className="mr-2 h-4 w-4" />
                    API Keys
                  </DropdownMenuItem>
                </DropdownMenuGroup>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  variant="destructive"
                  onClick={handleLogout}
                  disabled={logoutMutation.isPending}
                >
                  <LogOut className="mr-2 h-4 w-4" />
                  {logoutMutation.isPending ? "Signing out..." : "Sign out"}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        ) : (
          <p className="text-xs text-muted-foreground px-3">Everruns v{version}</p>
        )}
      </div>

      {/* Create Organisation Dialog */}
      <CreateOrganizationDialog open={createOrgOpen} onOpenChange={setCreateOrgOpen} />
    </div>
  );
}
