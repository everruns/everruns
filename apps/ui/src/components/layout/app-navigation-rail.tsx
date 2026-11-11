/**
 * Right-edge navigation for the chat-first layout (knowledge/ui/chat-mode.md).
 *
 * Decisions:
 * - Collapsed, it is an icon rail. System pages, Settings and Dev fold into one
 *   menu so the rail stays short; the person's profile menu sits at the bottom.
 * - Expanded, it is the regular sidebar without the Chat section (Chat is the
 *   left pane already). The choice is remembered on this device.
 * - It receives the shell's already-filtered sections: discovery never widens
 *   permissions or fetches destination content because someone hovered an icon.
 */
"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useId, type ReactElement } from "react";
import { PanelRightClose, PanelRightOpen, Search } from "lucide-react";
import { SettingsIcon } from "@/components/icons/facet-icons";
import { cn } from "@/lib/utils";
import { isDurableNavigationSection } from "@/lib/navigation";
import type { NavigationItem, NavigationSection } from "@/lib/navigation";
import { useCommandPalette } from "@/hooks/use-command-palette";
import { useAuth } from "@/providers/auth-provider";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Sidebar } from "./sidebar";
import { SidebarUserMenu } from "./sidebar-user-menu";
import { OverviewCountBadge } from "./sidebar-navigation";
import packageJson from "../../../package.json";

const { version } = packageJson;

const railButton =
  "relative flex h-10 w-12 shrink-0 items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring";

function isActive(item: NavigationItem, pathname: string) {
  const path = item.activePrefix ?? item.href;
  return item.exact ? pathname === path : pathname === path || pathname.startsWith(`${path}/`);
}

/** Settings, System and Dev: what the collapsed rail folds into one menu. */
export function isSystemMenuSection(section: NavigationSection) {
  return (
    isDurableNavigationSection(section) ||
    !!section.devOnly ||
    section.items.some((item) => item.activePrefix === "/settings")
  );
}

function RailHint({ label, children }: { label: string; children: ReactElement }) {
  const id = useId();
  return (
    <Tooltip>
      <TooltipTrigger asChild delay={150} aria-describedby={id}>
        {children}
      </TooltipTrigger>
      <TooltipContent id={id} role="tooltip" side="left" sideOffset={8}>
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

export function AppNavigationRail({
  sections,
  expanded,
  onExpandedChange,
}: {
  /** Visible sections, Chat excluded. */
  sections: NavigationSection[];
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
}) {
  const ui = useUiTranslations();
  const pathname = usePathname();
  const { setOpen: openCommandPalette } = useCommandPalette();
  const { user, requiresAuth, logout, logoutPending } = useAuth();

  if (expanded) {
    return (
      <nav aria-label={ui("Navigation")} className="chat-mode-rail relative border-l">
        <Sidebar config={{ navigation: sections }} className="w-full border-r-0" />
        <button
          type="button"
          aria-label={ui("Collapse navigation")}
          title={ui("Collapse navigation")}
          onClick={() => onExpandedChange(false)}
          className="absolute right-2 top-3 flex size-8 items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <PanelRightClose className="size-4" />
        </button>
      </nav>
    );
  }

  const primary = sections.filter((section) => !isSystemMenuSection(section));
  const system = sections.filter(isSystemMenuSection);
  const systemActive = system.some((section) =>
    section.items.some((item) => isActive(item, pathname)),
  );

  return (
    <nav
      aria-label={ui("Navigation")}
      className="chat-mode-rail items-center overflow-y-auto border-l bg-background"
    >
      <TooltipProvider delay={150}>
        <RailHint label={ui("Expand navigation")}>
          <button
            type="button"
            aria-label={ui("Expand navigation")}
            onClick={() => onExpandedChange(true)}
            className={railButton}
          >
            <PanelRightOpen className="size-4" />
          </button>
        </RailHint>
        <RailHint label={ui("Search (⌘K)")}>
          <button
            type="button"
            aria-label={ui("Search")}
            onClick={() => openCommandPalette(true)}
            className={railButton}
          >
            <Search className="size-4" />
          </button>
        </RailHint>
        {primary.map((section, index) => (
          <div
            key={section.id ?? section.label ?? index}
            role="group"
            aria-label={section.label ? ui(section.label) : undefined}
            className="flex flex-col items-center border-t py-1.5"
          >
            {section.items.map((item) => {
              const active = isActive(item, pathname);
              return (
                <RailHint key={item.href} label={ui(item.name)}>
                  <Link
                    href={item.href}
                    prefetch={false}
                    aria-label={ui(item.name)}
                    aria-current={active ? "page" : undefined}
                    className={cn(
                      railButton,
                      "border-l-2",
                      active ? "border-primary bg-muted text-foreground" : "border-transparent",
                    )}
                  >
                    <item.icon aria-hidden="true" className="icon-sharp size-4" />
                    {item.href === "/home" && (
                      <OverviewCountBadge className="absolute right-0.5 top-0.5 min-w-4 px-1 text-[10px]" />
                    )}
                  </Link>
                </RailHint>
              );
            })}
          </div>
        ))}
        {system.length > 0 && (
          <div className="flex flex-col items-center border-t py-1.5">
            <DropdownMenu modal={false}>
              <RailHint label={ui("System and settings")}>
                <DropdownMenuTrigger
                  aria-label={ui("System and settings")}
                  className={cn(
                    railButton,
                    "border-l-2",
                    systemActive ? "border-primary bg-muted text-foreground" : "border-transparent",
                  )}
                >
                  <SettingsIcon aria-hidden="true" className="icon-sharp size-4" />
                </DropdownMenuTrigger>
              </RailHint>
              <DropdownMenuPositioner side="left" align="start" sideOffset={8}>
                <DropdownMenuContent aria-label={ui("System and settings")} className="w-60">
                  {system.map((section, index) => (
                    <DropdownMenuGroup key={section.label ?? index}>
                      <DropdownMenuLabel>
                        {section.label ? ui(section.label) : ui("Organization")}
                      </DropdownMenuLabel>
                      {section.items.map((item) => (
                        <DropdownMenuItem
                          key={item.href}
                          label={ui(item.name)}
                          aria-current={isActive(item, pathname) ? "page" : undefined}
                          render={<Link href={item.href} prefetch={false} />}
                          className="gap-2"
                        >
                          <item.icon aria-hidden="true" className="icon-sharp size-4" />
                          {ui(item.name)}
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuGroup>
                  ))}
                </DropdownMenuContent>
              </DropdownMenuPositioner>
            </DropdownMenu>
          </div>
        )}
        <div className="mt-auto flex flex-col items-center border-t py-2">
          <SidebarUserMenu
            compact
            requiresAuth={requiresAuth}
            user={user ?? null}
            logout={logout}
            logoutPending={logoutPending}
            version={version}
          />
        </div>
      </TooltipProvider>
    </nav>
  );
}
