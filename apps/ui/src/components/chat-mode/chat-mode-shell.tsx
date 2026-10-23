"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { usePathname, useSearchParams } from "next/navigation";
import Link from "next/link";
import dynamic from "next/dynamic";
import { ArrowLeft, Eye, Maximize2, Search, X } from "lucide-react";
import { Sidebar, MobileSidebar } from "@/components/layout/sidebar";
import { useFeatureFlag, useFeatureFlags } from "@/providers/feature-flags-provider";
import { useOrg } from "@/providers/org-provider";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import { useCommandPalette } from "@/hooks/use-command-palette";
import {
  defaultNavigationSections,
  isDurableNavigationSection,
  visibleNavigationSections,
} from "@/lib/navigation";
import { usePolicies } from "@/hooks/use-policies";
import { useNavigationPolicy } from "@/hooks/use-navigation-policy";
import { SessionProvider } from "@/app/(main)/sessions/[sessionId]/session-context";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { chatPageRoute } from "@/lib/chat-page-context";
import {
  ChatModeContext,
  useChatMode,
  type ChatPageContext,
  type PageAction,
} from "./chat-mode-context";
import "./chat-mode.css";
import type { PageDraftAdapter } from "./page-draft";

const ChatPanel = dynamic(() =>
  import("@/components/chat/chat-panel").then((module) => module.ChatPanel),
);

export function ChatModeShell({ children }: { children: ReactNode }) {
  const enabled = useFeatureFlag("chat_mode");
  const { currentOrg } = useOrg();
  if (!enabled) {
    return (
      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <Sidebar className="hidden md:flex" />
        <MobileSidebar />
        <main className="min-w-0 flex-1 overflow-auto bg-background bg-brand-dots">{children}</main>
      </div>
    );
  }
  // No UI state, transcript, or page metadata crosses an organization switch.
  return <EnabledShell key={currentOrg?.public_id}>{children}</EnabledShell>;
}

function EnabledShell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const [open, setOpen] = useState(false);
  const [visited, setVisited] = useState(false);
  const [hint, setHint] = useState(true);
  const [registered, setRegistered] = useState<ChatPageContext | null>(null);
  const [adapter, setAdapter] = useState<PageDraftAdapter | undefined>();
  const [actions, setActions] = useState<PageAction[]>([]);
  const pageRef = useRef<HTMLElement>(null);
  const { thread, isLoading, error } = usePlatformChatThread({ ensure: visited });
  const { setOpen: setSearchOpen } = useCommandPalette();
  const route = chatPageRoute(pathname, searchParams);
  const page = useMemo<ChatPageContext>(
    () =>
      registered?.route === route ? registered : { route, label: pathname.split("/")[1] || "Home" },
    [registered, route, pathname],
  );
  const changeOpen = useCallback((value: boolean) => {
    setOpen(value);
    if (value) setVisited(true);
    else requestAnimationFrame(() => pageRef.current?.focus());
  }, []);
  const registerPage = useCallback((value: ChatPageContext, draftAdapter?: PageDraftAdapter) => {
    setRegistered(value);
    setAdapter(draftAdapter);
    return () => setRegistered((current) => (current === value ? null : current));
  }, []);
  const recordAction = useCallback(
    (label: string) => {
      setActions((current) => [...current.slice(-19), { id: crypto.randomUUID(), label, page }]);
    },
    [page],
  );

  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "j") {
        event.preventDefault();
        changeOpen(!open);
      }
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [changeOpen, open]);

  return (
    <ChatModeContext.Provider
      value={{
        pageContainer: pageRef,
        open,
        setOpen: changeOpen,
        page,
        registerPage,
        pageAdapter: registered?.route === route ? adapter : undefined,
        threadId: thread?.id,
        actions,
        recordAction,
      }}
    >
      <div className="chat-mode-shell min-h-0 flex-1" data-chat-mode={open ? "open" : "closed"}>
        <div className="chat-mode-sidebar min-h-0 overflow-hidden" inert={open} aria-hidden={open}>
          <Sidebar className="hidden md:flex" />
          <MobileSidebar />
        </div>
        <section
          className="chat-mode-conversation min-h-0 overflow-hidden border-r bg-background"
          aria-label="Platform Chat"
          hidden={!visited}
          inert={!open}
        >
          <div className="flex h-14 shrink-0 items-center justify-between border-b px-4">
            <h2 className="text-sm font-semibold">Chat</h2>
            <Button variant="ghost" size="sm" onClick={() => changeOpen(false)}>
              <ArrowLeft className="size-3.5" />
              Back to app
            </Button>
          </div>
          <div className="relative flex min-h-0 flex-1 flex-col">
            {hint && open && (
              <div className="absolute left-4 right-4 top-4 z-10 flex gap-3 border bg-card p-4 text-sm shadow-sm">
                <Eye className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <p>I can see the page beside this chat. Ask about it, or tell me what to change.</p>
                <button
                  type="button"
                  aria-label="Dismiss chat hint"
                  className="self-start"
                  onClick={() => setHint(false)}
                >
                  <X className="size-4" />
                </button>
              </div>
            )}
            {visited && thread ? (
              <SessionProvider key={thread.id} sessionId={thread.id}>
                <ChatPanel
                  pageAware
                  onPageMessage={() => setHint(false)}
                  showParticipants={false}
                  showRunCards
                />
              </SessionProvider>
            ) : (
              <p role={error ? "alert" : "status"} className="p-6 text-sm text-muted-foreground">
                {error
                  ? `Could not open Chat: ${error.message}`
                  : isLoading
                    ? "Opening Chat…"
                    : "Open Chat to begin."}
              </p>
            )}
          </div>
        </section>
        <section
          ref={pageRef}
          tabIndex={-1}
          className="chat-mode-page relative flex min-h-0 min-w-0 flex-col outline-none"
          aria-label="Current page"
        >
          {open && (
            <div className="flex h-10 shrink-0 items-center gap-2 border-b bg-background px-3 text-xs text-muted-foreground">
              <Eye className="size-3.5 shrink-0" />
              <span className="truncate">Showing this page to the chat</span>
              <button
                type="button"
                aria-label="Search pages"
                className="ml-auto p-1 hover:bg-muted"
                onClick={() => setSearchOpen(true)}
              >
                <Search className="size-3.5" />
              </button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Open full page"
                onClick={() => changeOpen(false)}
              >
                <Maximize2 className="size-3.5" />
              </Button>
            </div>
          )}
          <main className="chat-mode-page-content min-h-0 min-w-0 flex-1 overflow-auto bg-background bg-brand-dots">
            {children}
          </main>
        </section>
        <IconRail open={open} />
      </div>
    </ChatModeContext.Provider>
  );
}

function IconRail({ open }: { open: boolean }) {
  const mode = useChatMode();
  const pathname = usePathname();
  const flags = useFeatureFlags();
  const { hasRole } = useOrg();
  const durable = usePolicies("durable");
  const canNavigate = useNavigationPolicy();
  const sections = visibleNavigationSections(
    defaultNavigationSections.filter(
      (section) => !isDurableNavigationSection(section) || durable.can("durable.view"),
    ),
    flags,
    hasRole,
    process.env.NODE_ENV === "development",
    canNavigate,
  );
  return (
    <nav
      aria-label="Page navigation"
      aria-hidden={!open}
      className="chat-mode-rail overflow-y-auto border-l bg-background"
      inert={!open}
    >
      {sections.map((section, index) => (
        <div
          key={section.id ?? index}
          className={cn(
            "py-2",
            index > 0 && "border-t",
            section.items.some((item) => item.href === "/settings") && "mt-auto",
          )}
        >
          {section.items.map((item) => {
            if (item.href === "/chats")
              return (
                <button
                  key={item.href}
                  type="button"
                  aria-label="Close Chat mode"
                  title="Chat (⌘J)"
                  onClick={() => mode?.setOpen(false)}
                  className="flex h-10 w-12 items-center justify-center text-muted-foreground hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                >
                  <item.icon className="icon-sharp size-4" />
                </button>
              );
            const active =
              pathname === item.href ||
              (!item.exact && pathname.startsWith(`${item.activePrefix ?? item.href}/`));
            return (
              <Link
                key={item.href}
                href={item.href}
                prefetch={false}
                title={item.name}
                aria-label={item.name}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "flex h-10 w-12 items-center justify-center border-r-2 hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                  active
                    ? "border-primary bg-muted text-foreground"
                    : "border-transparent text-muted-foreground",
                )}
              >
                <item.icon className="icon-sharp size-4" />
              </Link>
            );
          })}
        </div>
      ))}
    </nav>
  );
}
