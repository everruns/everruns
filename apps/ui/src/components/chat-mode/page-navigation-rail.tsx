"use client";

import { useId, type ReactElement } from "react";
import Link from "next/link";
import { Menu } from "lucide-react";
import type { NavigationItem, NavigationSection } from "@/lib/navigation";
import { cn } from "@/lib/utils";
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

const railButton =
  "flex h-10 w-12 shrink-0 items-center justify-center hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring";

function isActive(item: NavigationItem, pathname: string) {
  const path = item.activePrefix ?? item.href;
  return pathname === path || (!item.exact && pathname.startsWith(`${path}/`));
}

export function PageNavigationHint({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: ReactElement;
}) {
  const id = useId();
  return (
    <Tooltip>
      <TooltipTrigger asChild delay={150} aria-describedby={id}>
        {children}
      </TooltipTrigger>
      <TooltipContent
        id={id}
        role="tooltip"
        side="left"
        sideOffset={8}
        className="max-w-64 py-2 motion-reduce:animate-none"
      >
        <p className="font-medium">{label}</p>
        {description && <p className="mt-1 text-xs text-muted-foreground">{description}</p>}
      </TooltipContent>
    </Tooltip>
  );
}

// Receive the shell's already-filtered model: discovery must never widen permissions
// or fetch destination content merely because someone hovered an icon.
export function PageNavigationRail({
  open,
  pathname,
  sections,
  onClose,
}: {
  open: boolean;
  pathname: string;
  sections: NavigationSection[];
  onClose: () => void;
}) {
  return (
    <nav
      aria-label="Page navigation"
      aria-hidden={!open}
      className="chat-mode-rail overflow-y-auto border-l bg-background"
      inert={!open}
    >
      {open && (
        <TooltipProvider delay={150}>
          <DropdownMenu modal={false}>
            <PageNavigationHint
              label="Browse pages"
              description="See page names and what each one is for."
            >
              <DropdownMenuTrigger
                aria-label="Browse pages"
                className={cn(railButton, "h-14 flex-col gap-1 border-b text-muted-foreground")}
              >
                <Menu aria-hidden="true" className="size-4" />
                <span className="text-[10px] font-medium">Pages</span>
              </DropdownMenuTrigger>
            </PageNavigationHint>
            <DropdownMenuPositioner side="left" align="start" sideOffset={8}>
              <DropdownMenuContent
                aria-label="Browse pages"
                className="w-80 max-w-[calc(100vw-4rem)] motion-reduce:animate-none"
              >
                {sections.map((section, index) => (
                  <DropdownMenuGroup key={section.id ?? index}>
                    {section.label && <DropdownMenuLabel>{section.label}</DropdownMenuLabel>}
                    {section.items.map((item) => {
                      const back = item.href === "/chats";
                      return (
                        <DropdownMenuItem
                          key={item.href}
                          aria-label={back ? "Back to app" : item.name}
                          label={back ? "Back to app" : item.name}
                          aria-current={!back && isActive(item, pathname) ? "page" : undefined}
                          render={back ? undefined : <Link href={item.href} prefetch={false} />}
                          onClick={back ? onClose : undefined}
                          className="items-start gap-3 py-2"
                        >
                          <item.icon aria-hidden="true" className="icon-sharp mt-0.5 size-4" />
                          <span>
                            <span className="block font-medium">
                              {back ? "Back to app" : item.name}
                            </span>
                            <span className="mt-0.5 block text-xs text-muted-foreground">
                              {back ? "Hide Chat and expand the current page." : item.description}
                            </span>
                          </span>
                        </DropdownMenuItem>
                      );
                    })}
                  </DropdownMenuGroup>
                ))}
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
          {sections.map((section, index) => (
            <div
              key={section.id ?? index}
              className={cn(
                "py-2",
                index > 0 && "border-t",
                section.items.some((item) => item.activePrefix === "/settings") && "mt-auto",
              )}
            >
              {section.items.map((item) => (
                <PageNavigationHint
                  key={item.href}
                  label={item.href === "/chats" ? "Back to app" : item.name}
                  description={
                    item.href === "/chats"
                      ? "Hide Chat and expand the current page."
                      : item.description
                  }
                >
                  {item.href === "/chats" ? (
                    <button
                      type="button"
                      aria-label="Close Chat mode"
                      onClick={onClose}
                      className={cn(railButton, "text-muted-foreground")}
                    >
                      <item.icon aria-hidden="true" className="icon-sharp size-4" />
                    </button>
                  ) : (
                    <Link
                      href={item.href}
                      prefetch={false}
                      aria-label={item.name}
                      aria-current={isActive(item, pathname) ? "page" : undefined}
                      className={cn(
                        railButton,
                        "border-r-2",
                        isActive(item, pathname)
                          ? "border-primary bg-muted text-foreground"
                          : "border-transparent text-muted-foreground",
                      )}
                    >
                      <item.icon aria-hidden="true" className="icon-sharp size-4" />
                    </Link>
                  )}
                </PageNavigationHint>
              ))}
            </div>
          ))}
        </TooltipProvider>
      )}
    </nav>
  );
}
