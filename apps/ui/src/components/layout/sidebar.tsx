/**
 * Composable sidebar with extension points.
 *
 * Design: The sidebar accepts an optional SidebarConfig to override navigation
 * sections, org actions, and inject extra sections. When no config is passed,
 * behavior is identical to the original hardcoded sidebar. This enables SaaS
 * forks to customize the sidebar in ~15 lines instead of duplicating 300+.
 *
 * Extension points:
 * - navigation: replace the default navigation sections entirely
 * - orgActions.createOrg: override "Create Organisation" click handler
 * - extraSections: append additional sections (billing, usage, etc.)
 */
"use client";

import { useState } from "react";
import packageJson from "../../../package.json";
import Link from "next/link";
import Image from "next/image";
import { usePathname, useRouter } from "next/navigation";
import { cn } from "@/lib/utils";
import { useAuth } from "@/providers/auth-provider";
import { useFeatureFlags } from "@/providers/feature-flags-provider";
import { useOrg } from "@/providers/org-provider";
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
  ChevronRight,
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
  ListTodo,
  Search,
} from "lucide-react";
import { capabilityIconMap } from "@/lib/capability-icons";
import { ExperimentalBadge } from "@/components/ui/experimental-badge";
import { useCommandPalette } from "@/hooks/use-command-palette";
import type { FeatureFlags } from "@/lib/api/types";

const isDev = process.env.NODE_ENV === "development";
const { version } = packageJson;

// --- Public types ---

export type NavigationItem = {
  name: string;
  href: string;
  icon: React.ComponentType<{ className?: string }>;
  /** Only show when this feature flag is enabled */
  flag?: keyof FeatureFlags;
  /** Use exact pathname match instead of prefix match */
  exact?: boolean;
  /** Show experimental badge */
  experimental?: boolean;
};

export type NavigationSection = {
  /** Section label shown above items. Omit for unlabeled sections. */
  label?: string;
  items: NavigationItem[];
  /** Only show in development mode */
  devOnly?: boolean;
  /** Start collapsed. Requires a label. */
  defaultCollapsed?: boolean;
};

export interface SidebarConfig {
  /** Replace default navigation sections entirely */
  navigation?: NavigationSection[];
  /** Override organization action handlers */
  orgActions?: {
    /** Override "Create Organisation" behavior. Return value unused. */
    createOrg?: () => void;
  };
  /** Append additional sections after the default ones */
  extraSections?: NavigationSection[];
}

// --- Default navigation (exported for reuse by SaaS) ---

export const defaultTopNavigation: NavigationItem[] = [
  { name: "Chat", href: "/chat", icon: MessageCircle, flag: "global_chat", experimental: true },
  { name: "Dashboard", href: "/dashboard", icon: LayoutDashboard },
  { name: "Sessions", href: "/sessions", icon: MessageSquare },
];

export const defaultBuildingBlocksNavigation: NavigationItem[] = [
  { name: "Harnesses", href: "/harnesses", icon: Shield },
  { name: "Agents", href: "/agents", icon: Boxes },
  { name: "Skills", href: "/skills", icon: BookOpen },
  { name: "Capabilities", href: "/capabilities", icon: Puzzle },
  { name: "MCP Servers", href: "/mcp-servers", icon: capabilityIconMap.mcp },
  { name: "Apps", href: "/apps", icon: Rocket, flag: "apps", experimental: true },
];

export const defaultBottomNavigation: NavigationItem[] = [
  { name: "Settings", href: "/settings", icon: Settings },
];

export const defaultDurableNavigation: NavigationItem[] = [
  { name: "Overview", href: "/durable", icon: Cog, exact: true },
  { name: "Workers", href: "/durable/workers", icon: Server },
  { name: "Workflows", href: "/durable/workflows", icon: Workflow },
  { name: "Queues", href: "/durable/queues", icon: ListTodo },
  { name: "Schedules", href: "/durable/schedules", icon: Calendar },
];

export const defaultDevNavigation: NavigationItem[] = [
  { name: "Dev Tools", href: "/dev", icon: FlaskConical },
];

export const defaultNavigationSections: NavigationSection[] = [
  { items: defaultTopNavigation },
  { label: "Building Blocks", items: defaultBuildingBlocksNavigation },
  { items: defaultBottomNavigation },
  { label: "Durable Execution", items: defaultDurableNavigation, defaultCollapsed: true },
  { label: "Dev", items: defaultDevNavigation, devOnly: true },
];

// --- Internal helpers ---

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

function NavLink({
  item,
  pathname,
  featureFlags,
}: {
  item: NavigationItem;
  pathname: string;
  featureFlags: FeatureFlags;
}) {
  if (item.flag && !featureFlags[item.flag]) return null;

  const isActive = item.exact
    ? pathname === item.href
    : pathname === item.href || pathname.startsWith(`${item.href}/`);

  return (
    <Link
      href={item.href}
      className={cn(
        "flex items-center gap-3 border-l-2 px-3 py-2.5 text-sm font-medium transition-colors",
        isActive
          ? "border-l-primary bg-card text-foreground"
          : "border-l-transparent text-muted-foreground hover:border-l-border hover:bg-card hover:text-foreground",
      )}
    >
      <item.icon className="icon-sharp h-[18px] w-[18px] shrink-0 stroke-[2.15]" />
      {item.name}
      {item.experimental && <ExperimentalBadge />}
    </Link>
  );
}

function NavSection({
  section,
  pathname,
  featureFlags,
  isFirst,
}: {
  section: NavigationSection;
  pathname: string;
  featureFlags: FeatureFlags;
  isFirst: boolean;
}) {
  const [collapsed, setCollapsed] = useState(section.defaultCollapsed ?? false);
  const toggle = () => setCollapsed((c) => !c);

  if (section.devOnly && !isDev) return null;

  const isCollapsible = section.defaultCollapsed !== undefined && section.label;

  return (
    <>
      {!isFirst && <div className="my-3 border-t" />}
      {section.label &&
        (isCollapsible ? (
          <button
            type="button"
            onClick={toggle}
            className="flex w-full items-center justify-between px-3 py-1 text-[10px] font-medium uppercase tracking-[0.22em] text-muted-foreground hover:text-foreground transition-colors"
          >
            {section.label}
            {collapsed ? (
              <ChevronRight className="h-3.5 w-3.5" />
            ) : (
              <ChevronDown className="h-3.5 w-3.5" />
            )}
          </button>
        ) : (
          <p className="px-3 py-1 text-[10px] font-medium uppercase tracking-[0.22em] text-muted-foreground">
            {section.label}
          </p>
        ))}
      {!collapsed &&
        section.items.map((item) => (
          <NavLink key={item.name} item={item} pathname={pathname} featureFlags={featureFlags} />
        ))}
    </>
  );
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

// --- Main component ---

export function Sidebar({ config }: { config?: Partial<SidebarConfig> }) {
  const pathname = usePathname();
  const router = useRouter();
  const {
    user,
    requiresAuth,
    logout,
    logoutPending,
    createOrganization: createOrgOverride,
  } = useAuth();
  const featureFlags = useFeatureFlags();
  const { currentOrg, organizations, setCurrentOrg } = useOrg();
  const [createOrgOpen, setCreateOrgOpen] = useState(false);

  const sections = config?.navigation ?? defaultNavigationSections;
  const allSections = config?.extraSections ? [...sections, ...config.extraSections] : sections;

  const { setOpen: openCommandPalette } = useCommandPalette();

  const handleCreateOrg =
    config?.orgActions?.createOrg ?? createOrgOverride ?? (() => setCreateOrgOpen(true));
  const useDefaultCreateOrgDialog = !config?.orgActions?.createOrg && !createOrgOverride;

  const handleLogout = async () => {
    await logout();
    router.push("/login");
  };

  // Track visible section index for separator logic
  let visibleIndex = 0;

  return (
    <div className="flex h-full w-64 flex-col border-r bg-background">
      {/* Logo */}
      <div className="flex h-16 items-center border-b bg-card px-6">
        <Link href="/dashboard" className="flex items-center gap-2">
          <Image src="/logo.svg" alt="Everruns" width={32} height={32} />
          <span className="text-xl font-bold">Everruns</span>
        </Link>
      </div>

      {/* Organization Selector */}
      {organizations.length > 0 && (
        <div className="border-b px-3 py-2">
          <DropdownMenu>
            <DropdownMenuTrigger className="flex w-full items-center gap-2 border border-transparent px-3 py-2 text-sm transition-colors hover:border-border hover:bg-card">
              <Building2 className="icon-sharp h-4 w-4 text-muted-foreground" />
              <span className="flex-1 text-left truncate font-medium">
                {currentOrg?.name ?? "Select Organization"}
              </span>
              <ChevronDown className="icon-sharp h-4 w-4 text-muted-foreground" />
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
                        <Check className="icon-sharp h-4 w-4 text-primary" />
                      )}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuGroup>
                <DropdownMenuSeparator />
                <DropdownMenuItem onClick={handleCreateOrg}>
                  <Plus className="icon-sharp mr-2 h-4 w-4" />
                  Create Organisation
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        </div>
      )}

      {/* Search trigger */}
      <div className="px-3 py-2">
        <button
          type="button"
          onClick={() => openCommandPalette(true)}
          className="flex w-full items-center gap-3 border border-input bg-transparent px-3 py-2 text-sm text-muted-foreground transition-colors hover:border-border hover:bg-card hover:text-foreground"
        >
          <Search className="h-4 w-4 shrink-0" />
          <span className="flex-1 text-left">Search...</span>
          <kbd className="hidden sm:inline-flex h-5 items-center gap-0.5 border bg-muted px-1.5 font-mono text-[10px] font-medium">
            <span className="text-xs">⌘</span>K
          </kbd>
        </button>
      </div>

      {/* Navigation */}
      <nav className="flex-1 min-h-0 overflow-y-auto space-y-1 bg-background py-4">
        {allSections.map((section, i) => {
          if (section.devOnly && !isDev) return null;
          const isFirst = visibleIndex === 0;
          visibleIndex++;
          return (
            <NavSection
              key={section.label ?? `section-${i}`}
              section={section}
              pathname={pathname}
              featureFlags={featureFlags}
              isFirst={isFirst}
            />
          );
        })}
      </nav>

      {/* User menu / Footer */}
      <div className="border-t p-3">
        {requiresAuth && user ? (
          <DropdownMenu>
            <DropdownMenuTrigger className="flex w-full items-center gap-3 border border-transparent px-3 py-2 text-sm transition-colors hover:border-border hover:bg-card">
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
              <ChevronUp className="icon-sharp h-4 w-4 text-muted-foreground" />
            </DropdownMenuTrigger>
            <DropdownMenuPositioner side="top" align="start">
              <DropdownMenuContent className="w-56">
                <DropdownMenuGroup>
                  <DropdownMenuLabel>My Account</DropdownMenuLabel>
                  <DropdownMenuItem onClick={() => router.push("/settings")}>
                    <User className="icon-sharp mr-2 h-4 w-4" />
                    Profile
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => router.push("/settings/api-keys")}>
                    <Key className="icon-sharp mr-2 h-4 w-4" />
                    API Keys
                  </DropdownMenuItem>
                </DropdownMenuGroup>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  variant="destructive"
                  onClick={handleLogout}
                  disabled={logoutPending}
                >
                  <LogOut className="icon-sharp mr-2 h-4 w-4" />
                  {logoutPending ? "Signing out..." : "Sign out"}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        ) : (
          <p className="text-xs text-muted-foreground px-3">Everruns v{version}</p>
        )}
      </div>

      {/* Create Organisation Dialog (only when using default handler) */}
      {useDefaultCreateOrgDialog && (
        <CreateOrganizationDialog open={createOrgOpen} onOpenChange={setCreateOrgOpen} />
      )}
    </div>
  );
}
