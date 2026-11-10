"use client";
import { ConfirmationProvider } from "@/components/ui/confirmation-provider";
import { useUiTranslations } from "@/providers/locale-provider";
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
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import dynamic from "next/dynamic";
import { ArrowLeft, Columns2, Eye, Maximize2, Search, X } from "lucide-react";
import { Sidebar, MobileSidebar } from "@/components/layout/sidebar";
import { useFeatureFlags, useFeatureFlagsState } from "@/providers/feature-flags-provider";
import { ChatSwitcher } from "@/components/chat/chat-switcher";
import { OPEN_CHAT_PARAM, useLandingRedirect, withoutOpenChat } from "./landing";
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
import { Button, LinkButton } from "@/components/ui/button";
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

const ChatWorkspace = dynamic(() =>
  import("@/components/chat/chat-workspace").then((module) => module.ChatWorkspace),
);
const ChatThreadView = dynamic(() =>
  import("@/components/chat/chat-thread-view").then((module) => module.ChatThreadView),
);

// Chat beside app pages is core, not an org opt-in (knowledge/ui/chat-mode.md):
// clicking Main chat on a page opens it beside that page, /chats is full screen.
export function ChatModeShell({
  children,
  navigation,
}: {
  children: ReactNode;
  /** Host-composed sections; the rail retains shared role, feature and policy gates. */
  navigation?: NavigationSection[];
}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const { isLoading: flagsLoading } = useFeatureFlagsState();
  useLandingRedirect(!orgLoading && !flagsLoading && !!currentOrg);
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
  const ui = useUiTranslations();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const chatRoute = pathname === "/chats" || pathname.startsWith("/chats/") || pathname === "/chat";
  const fullChat = chatRoute && pathname !== "/chats/history";
  const [open, setOpen] = useState(false);
  const splitOpen = open && !chatRoute;
  const conversationVisible = fullChat || splitOpen;
  const [activeChatId, setActiveChatId] = useState<string>();
  const [visited, setVisited] = useState(false);
  const [hint, setHint] = useState(true);
  // A thread opened beside the page replaces the page view (the page stays
  // mounted underneath so its drafts survive) until the person goes back.
  const [pageThreadId, setPageThreadId] = useState<string>();
  // Where "Show beside page" returns to from full-screen Chat.
  const lastPage = useRef<string>("/home");
  useEffect(() => {
    if (fullChat) setVisited(true);
  }, [fullChat]);
  useEffect(() => {
    if (chatRoute) return;
    const query = searchParams.toString();
    lastPage.current = query ? `${pathname}?${query}` : pathname;
    setPageThreadId(undefined);
  }, [chatRoute, pathname, searchParams]);
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
  const { thread, isLoading, error } = usePlatformChatThread({ ensure: visited || fullChat });
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

  const router = useRouter();
  const openOnArrival = searchParams.get(OPEN_CHAT_PARAM) === "open" && !chatRoute;
  useEffect(() => {
    if (!openOnArrival) return;
    changeOpen(true);
    router.replace(withoutOpenChat(pathname, searchParams), { scroll: false });
  }, [openOnArrival, changeOpen, router, pathname, searchParams]);

  const mainChatId = thread?.id;
  const selectedChatId = activeChatId ?? mainChatId;
  const openBeside = useCallback(
    (chatId?: string) => {
      setActiveChatId(chatId ?? mainChatId);
      changeOpen(true);
    },
    [changeOpen, mainChatId],
  );
  const showFullScreen = useCallback(() => {
    router.push(
      selectedChatId && selectedChatId !== mainChatId ? `/chats/${selectedChatId}` : "/chats",
    );
  }, [router, selectedChatId, mainChatId]);
  const showBesidePage = useCallback(() => {
    changeOpen(true);
    router.push(lastPage.current);
  }, [changeOpen, router]);
  const openThreadBeside = useCallback(
    (threadId: string) => {
      setPageThreadId(threadId);
      changeOpen(true);
    },
    [changeOpen],
  );

  useEffect(() => {
    // Cmd/Ctrl-J opens or closes Chat beside the page (on a Chat route it
    // focuses the composer). Cmd/Ctrl-Shift-J switches between full screen
    // and beside the page.
    const shortcut = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "j") return;
      event.preventDefault();
      if (event.shiftKey) {
        if (chatRoute) showBesidePage();
        else showFullScreen();
      } else if (chatRoute) {
        document
          .querySelector<HTMLElement>(
            '.chat-mode-conversation textarea, .chat-mode-conversation [contenteditable="true"]',
          )
          ?.focus();
      } else changeOpen(!open);
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [changeOpen, open, chatRoute, showBesidePage, showFullScreen]);

  return (
    <ChatModeContext.Provider
      value={{
        pageContainer: pageRef,
        pageContainerElement: pageElement,
        open: splitOpen,
        setOpen: changeOpen,
        page,
        registerPage,
        pageAdapter,
        pageViewAdapter: viewAdapter,
        threadId: activeChatId ?? thread?.id,
        openBeside,
        showFullScreen,
        showBesidePage,
        openThreadBeside,
        actions,
        recordAction,
      }}
    >
      <div
        className="chat-mode-shell min-h-0 flex-1"
        data-chat-mode={fullChat ? "chat" : splitOpen ? "open" : "closed"}
      >
        <div
          className="chat-mode-sidebar min-h-0 overflow-hidden"
          inert={splitOpen}
          aria-hidden={splitOpen}
        >
          <Sidebar className="hidden md:flex" />
          <MobileSidebar />
        </div>
        <section
          className="chat-mode-conversation min-h-0 overflow-hidden border-r bg-background"
          aria-label={ui("Platform Chat")}
          hidden={!visited && !fullChat}
          inert={!conversationVisible}
          aria-hidden={!conversationVisible}
        >
          {(visited || fullChat) && thread ? (
            <ChatWorkspace
              activeChatId={activeChatId}
              onActiveChatChange={setActiveChatId}
              visible={conversationVisible}
              pageAware={splitOpen}
              onPageMessage={() => setHint(false)}
              extraActions={
                splitOpen ? (
                  <>
                    <ChatSwitcher
                      mainChatId={thread.id}
                      activeChatId={activeChatId ?? thread.id}
                      onSelectChat={setActiveChatId}
                      onSelectThread={openThreadBeside}
                    />
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={ui("Full screen")}
                      title={ui("Full screen (⌘⇧J)")}
                      onClick={showFullScreen}
                    >
                      <Maximize2 className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={ui("Close chat")}
                      title={ui("Close chat (⌘J)")}
                      onClick={() => changeOpen(false)}
                    >
                      <X className="size-3.5" />
                    </Button>
                  </>
                ) : fullChat ? (
                  <Button
                    variant="outline"
                    size="sm"
                    title={ui("Show beside page (⌘⇧J)")}
                    onClick={showBesidePage}
                  >
                    <Columns2 className="size-3.5" /> {ui("Show beside page")}
                  </Button>
                ) : undefined
              }
              notice={
                hint && splitOpen ? (
                  <div className="flex shrink-0 gap-3 border-b bg-card px-4 py-3 text-xs">
                    <Eye className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                    <p>
                      {ui(
                        "I can see the page beside this chat. Ask about it, or tell me what to change.",
                      )}
                    </p>
                    <button
                      type="button"
                      aria-label={ui("Dismiss chat hint")}
                      className="ml-auto self-start"
                      onClick={() => setHint(false)}
                    >
                      <X className="size-4" />
                    </button>
                  </div>
                ) : undefined
              }
            />
          ) : (
            <p role={error ? "alert" : "status"} className="p-6 text-sm text-muted-foreground">
              {error
                ? ui("Could not open Chat: {value0}", { value0: error.message })
                : isLoading
                  ? ui("Opening Chat…")
                  : ui("Open Chat to begin.")}
            </p>
          )}
        </section>
        <PageViewRegistryContext.Provider value={viewRegistry}>
          <PageChangeReviewProvider>
            <section
              ref={pageContainerRef}
              tabIndex={-1}
              className="chat-mode-page relative flex min-h-0 min-w-0 flex-col outline-none"
              aria-label={ui("Current page")}
              inert={fullChat}
              aria-hidden={fullChat}
            >
              {splitOpen && (
                <div className="flex h-10 shrink-0 items-center gap-2 border-b bg-background px-3 text-xs text-muted-foreground">
                  <Eye className="size-3.5 shrink-0" />
                  <span className="truncate">{ui("Showing this page to the chat")}</span>
                  <PageNavigationHint
                    label={ui("Search pages")}
                    description={ui("Find a page or resource.")}
                  >
                    <button
                      type="button"
                      aria-label={ui("Search pages")}
                      className="ml-auto p-1 hover:bg-muted"
                      onClick={() => setSearchOpen(true)}
                    >
                      <Search className="size-3.5" />
                    </button>
                  </PageNavigationHint>
                  <PageNavigationHint
                    label={ui("Full page")}
                    description={ui("Hide Chat and expand the current page.")}
                  >
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={ui("Full page")}
                      onClick={() => changeOpen(false)}
                    >
                      <Maximize2 className="size-3.5" />
                    </Button>
                  </PageNavigationHint>
                </div>
              )}
              {splitOpen && pageThreadId && (
                <PageThread
                  threadId={pageThreadId}
                  coordinatorId={activeChatId ?? thread?.id}
                  backLabel={page.label}
                  onBack={() => setPageThreadId(undefined)}
                />
              )}
              <main
                hidden={splitOpen && !!pageThreadId}
                className="chat-mode-page-content min-h-0 min-w-0 flex-1 overflow-auto bg-background bg-brand-dots"
              >
                {fullChat ? null : (
                  <ConfirmationProvider
                    scope={`${currentOrg?.public_id}:${pathname}?${searchParams}`}
                  >
                    {children}
                  </ConfirmationProvider>
                )}
              </main>
              <SavedPageChanges />
            </section>
          </PageChangeReviewProvider>
        </PageViewRegistryContext.Provider>
        <IconRail open={splitOpen} navigation={navigation} />
      </div>
    </ChatModeContext.Provider>
  );
}

/** A thread opened from Chat while it sits beside a page: shown in the page pane. */
function PageThread({
  threadId,
  coordinatorId,
  backLabel,
  onBack,
}: {
  threadId: string;
  coordinatorId?: string;
  backLabel: string;
  onBack: () => void;
}) {
  const ui = useUiTranslations();
  return (
    <section
      aria-label={ui("Thread")}
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background"
    >
      <ChatThreadView
        threadId={threadId}
        coordinatorId={coordinatorId}
        threadMode
        extraActions={
          <>
            <Button variant="ghost" size="sm" onClick={onBack}>
              <ArrowLeft className="size-3.5" /> {backLabel}
            </Button>
            <LinkButton
              variant="ghost"
              size="icon-sm"
              href={`/chats/${threadId}`}
              aria-label={ui("Open thread full screen")}
            >
              <Maximize2 className="size-3.5" />
            </LinkButton>
          </>
        }
      />
    </section>
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
