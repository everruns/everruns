"use client";
import { PageChangeReviewProvider, SavedPageChanges } from "./page-change-review-provider";

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { usePathname, useSearchParams } from "next/navigation";
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
  type NavigationSection,
} from "@/lib/navigation";
import { usePolicies } from "@/hooks/use-policies";
import { useNavigationPolicy } from "@/hooks/use-navigation-policy";
import { SessionProvider } from "@/app/(main)/sessions/[sessionId]/session-context";
import { Button } from "@/components/ui/button";
import { chatPageRoute } from "@/lib/chat-page-context";
import {
  ChatModeContext,
  useChatMode,
  type ChatPageContext,
  type PageAction,
} from "./chat-mode-context";
import "./chat-mode.css";
import type { PageDraftAdapter } from "./page-draft";
import { PageNavigationHint, PageNavigationRail } from "./page-navigation-rail";
import { pageLocation } from "@/lib/chat-page-view";
import { usePageViewAdapter } from "./use-page-view-adapter";
import {
  createFormCollectionAdapter,
  formCollectionIdentity,
  type FormCollectionEntry,
} from "./form-collection-adapter";
import { PageViewRegistryContext, usePageViewRegistry } from "./page-view-state";

const ChatPanel = dynamic(() =>
  import("@/components/chat/chat-panel").then((module) => module.ChatPanel),
);

export function ChatModeShell({
  children,
  navigation,
}: {
  children: ReactNode;
  /** Host-composed sections; the rail retains shared role, feature and policy gates. */
  navigation?: NavigationSection[];
}) {
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
  return (
    <EnabledShell key={currentOrg?.public_id} navigation={navigation}>
      {children}
    </EnabledShell>
  );
}

function EnabledShell({
  children,
  navigation,
}: {
  children: ReactNode;
  navigation?: NavigationSection[];
}) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const [open, setOpen] = useState(false);
  const [visited, setVisited] = useState(false);
  const [hint, setHint] = useState(true);
  const [registrations, setRegistrations] = useState<
    Array<{
      token: string;
      page: ChatPageContext;
      adapter?: PageDraftAdapter;
      priority: number;
    }>
  >([]);
  const [actions, setActions] = useState<PageAction[]>([]);
  const pageRef = useRef<HTMLElement>(null);
  const [pageElement, setPageElement] = useState<HTMLElement | null>(null);
  const pageContainerRef = useCallback((element: HTMLElement | null) => {
    pageRef.current = element;
    setPageElement(element);
  }, []);
  const { thread, isLoading, error } = usePlatformChatThread({ ensure: visited });
  const { setOpen: setSearchOpen } = useCommandPalette();
  const route = chatPageRoute(pathname, searchParams);
  const { currentOrg } = useOrg();
  const registered = useMemo(
    () =>
      registrations
        .filter((item) => item.page.route === route)
        .sort((a, b) => a.priority - b.priority)
        .at(-1),
    [registrations, route],
  );
  const page = useMemo<ChatPageContext>(() => {
    const primary = registered?.page ?? pageLocation(route);
    const peers = registrations.filter(
      (item) => item.page.route === route && item.adapter && item.priority === registered?.priority,
    );
    if (peers.length < 2) return primary;
    // A collection reads all peer forms. Its chip should identify the page,
    // rather than whichever form happened to register last.
    const title = registrations
      .filter((item) => item.page.route === route && !item.adapter)
      .sort((a, b) => a.priority - b.priority)
      .at(-1)?.page.label;
    return { ...primary, label: title ?? pageLocation(route).label };
  }, [registered, registrations, route]);
  const { registry: viewRegistry, read: readViewState } = usePageViewRegistry(route);
  const viewAdapter = usePageViewAdapter(page, currentOrg?.public_id, () => readViewState(route));
  const activeForms = useMemo(() => {
    const forms = registrations.filter((item) => item.page.route === route && item.adapter);
    const priority = Math.max(...forms.map((item) => item.priority));
    const active = forms.filter((item) => item.priority === priority);
    active.sort((a, b) =>
      (a.page.form_name ?? a.page.entity_type ?? a.token).localeCompare(
        b.page.form_name ?? b.page.entity_type ?? b.token,
      ),
    );
    const names = new Map<string, number>();
    return active.map((item) => {
      const base = item.page.form_name ?? item.page.entity_type ?? "form";
      const index = names.get(base) ?? 0;
      names.set(base, index + 1);
      return {
        key: index ? `${base}_${index + 1}` : base,
        label: item.page.label,
        identity: item.adapter!.binding ?? item.token,
        adapter: item.adapter!,
      };
    });
  }, [registrations, route]);
  const formsRef = useRef<FormCollectionEntry[]>(activeForms);
  useLayoutEffect(() => {
    formsRef.current = activeForms;
  }, [activeForms]);
  const getForms = useCallback(() => formsRef.current, []);
  const formsKey = formCollectionIdentity(activeForms);
  const singleAdapter = activeForms.length === 1 ? activeForms[0].adapter : undefined;
  const pageAdapter = useMemo(
    () =>
      singleAdapter ??
      (formsKey === "[]"
        ? viewAdapter
        : createFormCollectionAdapter({ entries: getForms, identity: formsKey })),
    [singleAdapter, formsKey, viewAdapter, getForms],
  );
  const changeOpen = useCallback((value: boolean) => {
    setOpen(value);
    if (value) setVisited(true);
    else requestAnimationFrame(() => pageRef.current?.focus());
  }, []);
  const registerPage = useCallback(
    (value: ChatPageContext, draftAdapter?: PageDraftAdapter, priority = draftAdapter ? 10 : 0) => {
      const token = crypto.randomUUID();
      setRegistrations((current) => [
        ...current,
        { token, page: value, adapter: draftAdapter, priority },
      ]);
      return () => setRegistrations((current) => current.filter((item) => item.token !== token));
    },
    [],
  );
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
        pageContainerElement: pageElement,
        open,
        setOpen: changeOpen,
        page,
        registerPage,
        pageAdapter,
        pageViewAdapter: viewAdapter,
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
        <PageViewRegistryContext.Provider value={viewRegistry}>
          <PageChangeReviewProvider>
            <section
              ref={pageContainerRef}
              tabIndex={-1}
              className="chat-mode-page relative flex min-h-0 min-w-0 flex-col outline-none"
              aria-label="Current page"
            >
              {open && (
                <div className="flex h-10 shrink-0 items-center gap-2 border-b bg-background px-3 text-xs text-muted-foreground">
                  <Eye className="size-3.5 shrink-0" />
                  <span className="truncate">Showing this page to the chat</span>
                  <PageNavigationHint label="Search pages" description="Find a page or resource.">
                    <button
                      type="button"
                      aria-label="Search pages"
                      className="ml-auto p-1 hover:bg-muted"
                      onClick={() => setSearchOpen(true)}
                    >
                      <Search className="size-3.5" />
                    </button>
                  </PageNavigationHint>
                  <PageNavigationHint
                    label="Open full page"
                    description="Hide Chat and expand the current page."
                  >
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label="Open full page"
                      onClick={() => changeOpen(false)}
                    >
                      <Maximize2 className="size-3.5" />
                    </Button>
                  </PageNavigationHint>
                </div>
              )}
              <main className="chat-mode-page-content min-h-0 min-w-0 flex-1 overflow-auto bg-background bg-brand-dots">
                {children}
              </main>
              <SavedPageChanges />
            </section>
          </PageChangeReviewProvider>
        </PageViewRegistryContext.Provider>
        <IconRail open={open} navigation={navigation} />
      </div>
    </ChatModeContext.Provider>
  );
}

function IconRail({
  open,
  navigation = defaultNavigationSections,
}: {
  open: boolean;
  navigation?: NavigationSection[];
}) {
  const mode = useChatMode();
  const pathname = usePathname();
  const flags = useFeatureFlags();
  const { hasRole } = useOrg();
  const durable = usePolicies("durable");
  const canNavigate = useNavigationPolicy();
  const sections = visibleNavigationSections(
    navigation.filter(
      (section) => !isDurableNavigationSection(section) || durable.can("durable.view"),
    ),
    flags,
    hasRole,
    process.env.NODE_ENV === "development",
    canNavigate,
  );
  return (
    <PageNavigationRail
      open={open}
      pathname={pathname}
      sections={sections}
      onClose={() => mode?.setOpen(false)}
    />
  );
}
