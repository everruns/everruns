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
  useSyncExternalStore,
  Suspense,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import dynamic from "next/dynamic";
import {
  Columns2,
  GripHorizontal,
  Maximize2,
  MessageSquare,
  PanelRight,
  PictureInPicture2,
  X,
} from "lucide-react";
import { Sidebar, MobileSidebar } from "@/components/layout/sidebar";
import { useFeatureFlags, useFeatureFlagsState } from "@/providers/feature-flags-provider";
import { ChatSwitcher } from "@/components/chat/chat-switcher";
import { useLandingRedirect } from "./landing";
import {
  useClassicChatPreference,
  useLayoutPreference,
  type AppLayout,
} from "@/hooks/use-layout-preference";
import { AppNavigationRail } from "@/components/layout/app-navigation-rail";
import { useOrg } from "@/providers/org-provider";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import {
  defaultNavigationSections,
  isDurableNavigationSection,
  visibleNavigationSections,
  type NavigationSection,
} from "@/lib/navigation";
import { usePolicies } from "@/hooks/use-policies";
import { useNavigationPolicy } from "@/hooks/use-navigation-policy";
import { Button } from "@/components/ui/button";
import { chatPageRoute } from "@/lib/chat-page-context";
import { ChatModeContext, type ChatPageContext, type PageAction } from "./chat-mode-context";
import "./chat-mode.css";
import type { PageDraftAdapter } from "./page-draft";
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

// Classic is the default layout (knowledge/ui/chat-mode.md): the left sidebar,
// with Chat full screen, floating over the page or docked on its right (a
// second personal preference). Chat first puts Main chat on the left, the page
// on the right and navigation in a rail on the right edge. The URL alone says
// which page is shown, so browser back and forward step through pages and
// threads; whether floating or docked Chat is open is per browser tab.
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
  const { layout, ready: layoutReady } = useLayoutPreference();
  useLandingRedirect(!orgLoading && !flagsLoading && !!currentOrg && layoutReady, layout);
  // No UI state, transcript, or page metadata crosses an organization switch.
  return (
    <EnabledShell key={currentOrg?.public_id} navigation={navigation} layout={layout}>
      {children}
    </EnabledShell>
  );
}

const DESKTOP_QUERY = "(min-width: 768px)";
const subscribeDesktop = (notify: () => void) => {
  if (typeof window.matchMedia !== "function") return () => {};
  const query = window.matchMedia(DESKTOP_QUERY);
  query.addEventListener?.("change", notify);
  return () => query.removeEventListener?.("change", notify);
};
const readDesktop = () =>
  typeof window.matchMedia === "function" ? window.matchMedia(DESKTOP_QUERY).matches : true;

const NAV_EXPANDED_KEY = "everruns.nav-expanded";
const ACTIVE_CHAT_KEY = "everruns.active-chat";
const INLINE_OPEN_KEY = "everruns.classic-chat-open";

function readStorage(key: string) {
  try {
    return window.sessionStorage.getItem(key) ?? undefined;
  } catch {
    return undefined;
  }
}

function EnabledShell({
  children,
  navigation,
  layout,
}: {
  children: ReactNode;
  navigation?: NavigationSection[];
  layout: AppLayout;
}) {
  const ui = useUiTranslations();
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const desktop = useSyncExternalStore(subscribeDesktop, readDesktop, () => true);
  const chatRoute = pathname === "/chats" || pathname.startsWith("/chats/") || pathname === "/chat";
  const fullChat = chatRoute && pathname !== "/chats/history";
  // On small screens both layouts show one pane at a time, like classic.
  const chatFirst = layout === "chat_first" && desktop;
  const beside = chatFirst && !chatRoute;
  const { classicChat, setClassicChat } = useClassicChatPreference();
  // Floating or docked Chat in Classic; small screens keep one pane at a time.
  const inlineMode =
    !chatFirst && desktop && classicChat !== "full_screen" ? classicChat : undefined;
  const [inlineOpen, setInlineOpenState] = useState(() => readStorage(INLINE_OPEN_KEY) === "1");
  const setInlineOpen = useCallback((value: boolean) => {
    setInlineOpenState(value);
    try {
      window.sessionStorage.setItem(INLINE_OPEN_KEY, value ? "1" : "0");
    } catch {
      // Storage disabled: the choice lasts until reload.
    }
  }, []);
  const inline = !!inlineMode && inlineOpen && !chatRoute;
  const pageAware = beside || inline;
  const conversationVisible = fullChat || pageAware;
  const [activeChatId, setActiveChatIdState] = useState<string | undefined>(() =>
    readStorage(ACTIVE_CHAT_KEY),
  );
  const setActiveChatId = useCallback((id: string | undefined) => {
    setActiveChatIdState(id);
    try {
      if (id) window.sessionStorage.setItem(ACTIVE_CHAT_KEY, id);
      else window.sessionStorage.removeItem(ACTIVE_CHAT_KEY);
    } catch {
      // Storage disabled: the choice lasts until reload.
    }
  }, []);
  // "New side chat" in the switcher drafts beside the page instead of going to
  // full-screen /chats/new. 0 is no draft; each new draft gets a new
  // number so it starts empty. Not stored: an unsent draft has no session yet.
  const [newChat, setNewChat] = useState(0);
  const [visited, setVisited] = useState(false);
  const [navExpanded, setNavExpandedState] = useState(() => {
    try {
      return window.localStorage.getItem(NAV_EXPANDED_KEY) === "1";
    } catch {
      return false;
    }
  });
  const setNavExpanded = useCallback((value: boolean) => {
    setNavExpandedState(value);
    try {
      window.localStorage.setItem(NAV_EXPANDED_KEY, value ? "1" : "0");
    } catch {
      // Storage disabled: the choice lasts until reload.
    }
  }, []);
  // Docked Chat shares the left sidebar's row, so the page keeps its minimum
  // after the sidebar rather than after the rail.
  const reserved = inlineMode === "docked" ? SIDEBAR_WIDTH : RAIL_WIDTH;
  const {
    width: chatWidth,
    setWidth: setChatWidth,
    reset: resetChatWidth,
  } = useChatWidth(reserved);
  const floating = useFloatingBox();
  // Where "Show beside page" and ⌘⇧J return to from full-screen Chat.
  const lastPage = useRef<string>("/home");
  useEffect(() => {
    if (conversationVisible) setVisited(true);
  }, [conversationVisible]);
  useEffect(() => {
    if (chatRoute) return;
    const query = searchParams.toString();
    lastPage.current = query ? `${pathname}?${query}` : pathname;
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
  const { thread, isLoading, error } = usePlatformChatThread({
    ensure: visited || conversationVisible,
  });
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

  const mainChatId = thread?.id;
  const independentChat =
    (fullChat && pathname.startsWith("/chats/")) || (!chatRoute && (!!newChat || !!activeChatId));
  // A remembered side chat from another organization or a deleted one falls
  // back to Main chat inside the workspace.
  const selectedChatId = activeChatId ?? mainChatId;
  const chatHref = useCallback(
    (chatId?: string) => (chatId && chatId !== mainChatId ? `/chats/${chatId}` : "/chats"),
    [mainChatId],
  );
  const showFullScreen = useCallback(() => {
    if (newChat) {
      setNewChat(0);
      router.push("/chats/new");
    } else router.push(chatHref(selectedChatId));
  }, [router, chatHref, selectedChatId, newChat]);
  const selectChat = useCallback(
    (chatId: string) => {
      setNewChat(0);
      setActiveChatId(chatId === mainChatId ? undefined : chatId);
    },
    [setActiveChatId, mainChatId],
  );
  const startNewChat = useCallback(() => setNewChat((current) => current + 1), []);
  const showBesidePage = useCallback(() => {
    if (inlineMode) setInlineOpen(true);
    router.push(lastPage.current);
  }, [router, inlineMode, setInlineOpen]);
  const openBeside = useCallback(
    (chatId?: string) => {
      setNewChat(0);
      setActiveChatId(chatId === mainChatId ? undefined : chatId);
      if (inlineMode) setInlineOpen(true);
      if (chatRoute) router.push(lastPage.current);
    },
    [setActiveChatId, mainChatId, chatRoute, router, inlineMode, setInlineOpen],
  );
  const focusComposer = useCallback(
    () =>
      document
        .querySelector<HTMLElement>(
          '.chat-mode-conversation textarea, .chat-mode-conversation [contenteditable="true"]',
        )
        ?.focus(),
    [],
  );
  // Floating or docked Chat is shown on the next render; focus after it paints.
  const showInline = useCallback(() => {
    setInlineOpen(true);
    requestAnimationFrame(focusComposer);
  }, [setInlineOpen, focusComposer]);
  const closeInline = useCallback(() => {
    setInlineOpen(false);
    requestAnimationFrame(() => pageRef.current?.focus());
  }, [setInlineOpen]);
  const openThreadBeside = useCallback(
    (threadId: string) => router.push(`/threads/${threadId}`),
    [router],
  );
  const setOpen = useCallback(
    (value: boolean) => {
      if (value && (chatFirst || inlineMode)) {
        if (chatRoute) showBesidePage();
        else if (inlineMode) setInlineOpen(true);
      } else if (value) router.push(chatHref(selectedChatId));
      else if (chatRoute) showBesidePage();
      else if (inlineMode) closeInline();
      else requestAnimationFrame(() => pageRef.current?.focus());
    },
    [
      chatFirst,
      inlineMode,
      chatRoute,
      showBesidePage,
      setInlineOpen,
      closeInline,
      router,
      chatHref,
      selectedChatId,
    ],
  );

  useEffect(() => {
    // Cmd/Ctrl-J: chat-first focuses the composer; floating or docked Chat in
    // Classic shows or hides; otherwise classic opens or closes full-screen
    // Chat. Cmd/Ctrl-Shift-J switches full-screen Chat and the page.
    const shortcut = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "j") return;
      event.preventDefault();
      if (!event.shiftKey && chatFirst) focusComposer();
      else if (!event.shiftKey && inlineMode && !chatRoute) {
        if (inline) closeInline();
        else showInline();
      } else if (chatRoute) showBesidePage();
      else showFullScreen();
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [
    chatFirst,
    chatRoute,
    inlineMode,
    inline,
    closeInline,
    showInline,
    focusComposer,
    showBesidePage,
    showFullScreen,
  ]);

  const flags = useFeatureFlags();
  const { hasRole } = useOrg();
  const durable = usePolicies("durable");
  const canNavigate = useNavigationPolicy();
  const railSections = useMemo(
    () =>
      visibleNavigationSections(
        (navigation ?? defaultNavigationSections).filter(
          (section) =>
            section.id !== "chats" &&
            (!isDurableNavigationSection(section) || durable.can("durable.view")),
        ),
        flags,
        hasRole,
        process.env.NODE_ENV === "development",
        canNavigate,
      ),
    [navigation, durable, flags, hasRole, canNavigate],
  );

  const mode = chatFirst
    ? fullChat
      ? "chat"
      : "beside"
    : fullChat
      ? "classic-chat"
      : inline
        ? `classic-${inlineMode}`
        : "classic";
  return (
    <ChatModeContext.Provider
      value={{
        pageContainer: pageRef,
        pageContainerElement: pageElement,
        open: pageAware,
        setOpen,
        page,
        registerPage,
        pageAdapter,
        pageViewAdapter: viewAdapter,
        threadId: selectedChatId,
        openBeside: chatFirst || inlineMode ? openBeside : undefined,
        showFullScreen,
        showBesidePage: chatFirst || inlineMode ? showBesidePage : undefined,
        openThreadBeside: pageAware ? openThreadBeside : undefined,
        actions,
        recordAction,
      }}
    >
      <div
        className="chat-mode-shell min-h-0 flex-1"
        data-layout={mode}
        data-nav={navExpanded ? "expanded" : "rail"}
        style={
          {
            "--chat-width": `${chatWidth}px`,
            "--float-right": `${floating.box.right}px`,
            "--float-bottom": `${floating.box.bottom}px`,
            "--float-width": `${floating.box.width}px`,
            "--float-height": `${floating.box.height}px`,
          } as CSSProperties
        }
      >
        {!chatFirst && (
          <div className="chat-mode-sidebar min-h-0 overflow-hidden">
            <Sidebar className="hidden md:flex" />
            <MobileSidebar />
          </div>
        )}
        <section
          className="chat-mode-conversation min-h-0 overflow-hidden bg-background"
          aria-label={ui("Platform Chat")}
          data-inline={inline ? inlineMode : undefined}
          hidden={!visited && !conversationVisible}
          inert={!conversationVisible}
          aria-hidden={!conversationVisible}
        >
          {(visited || conversationVisible) && (thread || independentChat) ? (
            // Its own boundary: Chat loads beside the page on arrival, and a
            // suspended conversation must not hold back the page's navigations
            // (a server redirect or router.replace would otherwise never commit).
            <Suspense
              fallback={
                <p role="status" className="p-6 text-sm text-muted-foreground">
                  {ui("Opening Chat…")}
                </p>
              }
            >
              <ChatWorkspace
                activeChatId={activeChatId}
                onActiveChatChange={setActiveChatId}
                newChat={newChat}
                onNewChatCreated={selectChat}
                visible={conversationVisible}
                pageAware={pageAware}
                extraActions={
                  inline ? (
                    <>
                      {inlineMode === "floating" && (
                        <FloatingMoveHandle onMove={floating.move} onNudge={floating.nudge} />
                      )}
                      {thread && (
                        <ChatSwitcher
                          mainChatId={thread.id}
                          activeChatId={selectedChatId ?? thread.id}
                          drafting={!!newChat}
                          onSelectChat={selectChat}
                          onSelectThread={openThreadBeside}
                          onNewChat={startNewChat}
                        />
                      )}
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={
                          inlineMode === "floating" ? ui("Dock to the right") : ui("Float")
                        }
                        title={inlineMode === "floating" ? ui("Dock to the right") : ui("Float")}
                        onClick={() =>
                          setClassicChat(inlineMode === "floating" ? "docked" : "floating")
                        }
                      >
                        {inlineMode === "floating" ? (
                          <PanelRight className="size-3.5" />
                        ) : (
                          <PictureInPicture2 className="size-3.5" />
                        )}
                      </Button>
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
                        aria-label={ui("Hide chat")}
                        title={ui("Hide chat (⌘J)")}
                        onClick={closeInline}
                      >
                        <X className="size-3.5" />
                      </Button>
                    </>
                  ) : beside ? (
                    <>
                      {thread && (
                        <ChatSwitcher
                          mainChatId={thread.id}
                          activeChatId={selectedChatId ?? thread.id}
                          drafting={!!newChat}
                          onSelectChat={selectChat}
                          onSelectThread={openThreadBeside}
                          onNewChat={startNewChat}
                        />
                      )}
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={ui("Full screen")}
                        title={ui("Full screen (⌘⇧J)")}
                        onClick={showFullScreen}
                      >
                        <Maximize2 className="size-3.5" />
                      </Button>
                    </>
                  ) : fullChat ? (
                    <Button
                      variant="outline"
                      size="sm"
                      title={ui("Back to page (⌘⇧J)")}
                      onClick={showBesidePage}
                    >
                      <Columns2 className="size-3.5" />{" "}
                      {chatFirst || inlineMode ? ui("Show beside page") : ui("Back to page")}
                    </Button>
                  ) : undefined
                }
              />
            </Suspense>
          ) : (
            <p role={error ? "alert" : "status"} className="p-6 text-sm text-muted-foreground">
              {error
                ? ui("Could not open Chat: {value0}", { value0: error.message })
                : isLoading
                  ? ui("Opening Chat…")
                  : ui("Open Chat to begin.")}
            </p>
          )}
          {inline && inlineMode === "floating" && (
            <FloatingResizeHandle onResize={floating.resize} />
          )}
        </section>
        {(beside || (inline && inlineMode === "docked")) && (
          <ChatSplitter
            side={beside ? "left" : "right"}
            reserved={reserved}
            width={chatWidth}
            onWidthChange={setChatWidth}
            onReset={resetChatWidth}
            onCollapsePage={showFullScreen}
          />
        )}
        {inlineMode && !inline && !chatRoute && (
          <Button
            className="chat-mode-launcher"
            aria-label={ui("Show chat")}
            title={ui("Show chat (⌘J)")}
            onClick={showInline}
          >
            <MessageSquare className="size-4" /> {ui("Chat")}
            <kbd className="text-[10px] opacity-70">{ui("⌘J")}</kbd>
          </Button>
        )}
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
              <main className="chat-mode-page-content min-h-0 min-w-0 flex-1 overflow-auto bg-background bg-brand-dots">
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
        {chatFirst && (
          <AppNavigationRail
            sections={railSections}
            expanded={navExpanded}
            onExpandedChange={setNavExpanded}
          />
        )}
      </div>
    </ChatModeContext.Provider>
  );
}

const CHAT_WIDTH_KEY = "everruns.chat-width";
const CHAT_MIN = 360;
const PAGE_MIN = 480;
const RAIL_WIDTH = 56;
const SIDEBAR_WIDTH = 240;
const defaultChatWidth = () =>
  typeof window === "undefined" ? 560 : Math.round(Math.min(600, window.innerWidth * 0.4));

/**
 * Chat pane width, remembered on this device and kept within both panes'
 * minimums. `reserved` is the fixed navigation beside the two panes.
 */
function useChatWidth(reserved: number) {
  const [stored, setStored] = useState<number | undefined>(() => {
    try {
      const value = Number(window.localStorage.getItem(CHAT_WIDTH_KEY));
      return Number.isFinite(value) && value > 0 ? value : undefined;
    } catch {
      return undefined;
    }
  });
  const width = clampChatWidth(stored ?? defaultChatWidth(), undefined, reserved);
  const setWidth = useCallback(
    (value: number) => {
      const next = clampChatWidth(value, undefined, reserved);
      setStored(next);
      try {
        window.localStorage.setItem(CHAT_WIDTH_KEY, String(next));
      } catch {
        // Storage disabled: the width lasts until reload.
      }
    },
    [reserved],
  );
  const reset = useCallback(() => {
    setStored(undefined);
    try {
      window.localStorage.removeItem(CHAT_WIDTH_KEY);
    } catch {
      // Storage disabled.
    }
  }, []);
  return { width, setWidth, reset };
}

export function clampChatWidth(
  value: number,
  viewport = typeof window === "undefined" ? 1440 : window.innerWidth,
  reserved = RAIL_WIDTH,
) {
  return Math.round(Math.max(CHAT_MIN, Math.min(value, viewport - PAGE_MIN - reserved)));
}

/**
 * Visible divider between Chat and the page: a bordered gutter with a grip
 * that turns primary on hover or focus. Drag or use the arrow keys to resize;
 * double-click resets. Dragging the page narrower than half its minimum
 * switches to full-screen Chat. `side` is where Chat sits: left in Chat first,
 * right when docked in Classic.
 */
function ChatSplitter({
  side,
  reserved,
  width,
  onWidthChange,
  onReset,
  onCollapsePage,
}: {
  side: "left" | "right";
  reserved: number;
  width: number;
  onWidthChange: (width: number) => void;
  onReset: () => void;
  onCollapsePage: () => void;
}) {
  const ui = useUiTranslations();
  const [dragging, setDragging] = useState(false);
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={ui("Resize chat")}
      aria-valuenow={width}
      aria-valuemin={CHAT_MIN}
      tabIndex={0}
      data-dragging={dragging || undefined}
      className="chat-mode-splitter"
      onDoubleClick={onReset}
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
          event.preventDefault();
          // The arrow moves the divider; Chat grows on the side it moves away from.
          const towardChat = (event.key === "ArrowLeft") === (side === "left");
          onWidthChange(width + (towardChat ? -32 : 32));
        }
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        const target = event.currentTarget;
        target.setPointerCapture?.(event.pointerId);
        const bounds = target.parentElement?.getBoundingClientRect();
        const left = bounds?.left ?? 0;
        const right = bounds?.right ?? window.innerWidth;
        setDragging(true);
        const move = (moveEvent: PointerEvent) =>
          onWidthChange(side === "left" ? moveEvent.clientX - left : right - moveEvent.clientX);
        const up = (upEvent: PointerEvent) => {
          setDragging(false);
          target.removeEventListener("pointermove", move);
          target.removeEventListener("pointerup", up);
          target.removeEventListener("pointercancel", up);
          const pageLeft =
            side === "left"
              ? window.innerWidth - upEvent.clientX - reserved
              : upEvent.clientX - left - reserved;
          if (pageLeft < PAGE_MIN / 2) onCollapsePage();
        };
        target.addEventListener("pointermove", move);
        target.addEventListener("pointerup", up);
        target.addEventListener("pointercancel", up);
      }}
    />
  );
}

const FLOAT_KEY = "everruns.chat-float";
const FLOAT_MARGIN = 16;
const FLOAT_MIN = { width: 340, height: 360 };
interface FloatingBox {
  right: number;
  bottom: number;
  width: number;
  height: number;
}
const defaultFloatingBox = (): FloatingBox => ({
  right: 24,
  bottom: 24,
  width: 480,
  height: typeof window === "undefined" ? 600 : Math.min(640, window.innerHeight - 96),
});

/** Keeps the floating card on screen and at least its minimum size. */
export function clampFloatingBox(
  box: FloatingBox,
  viewport = typeof window === "undefined"
    ? { width: 1440, height: 900 }
    : { width: window.innerWidth, height: window.innerHeight },
): FloatingBox {
  const width = Math.round(
    Math.max(FLOAT_MIN.width, Math.min(box.width, viewport.width - 2 * FLOAT_MARGIN)),
  );
  const height = Math.round(
    Math.max(FLOAT_MIN.height, Math.min(box.height, viewport.height - 2 * FLOAT_MARGIN)),
  );
  return {
    width,
    height,
    right: Math.round(
      Math.max(FLOAT_MARGIN, Math.min(box.right, viewport.width - width - FLOAT_MARGIN)),
    ),
    bottom: Math.round(
      Math.max(FLOAT_MARGIN, Math.min(box.bottom, viewport.height - height - FLOAT_MARGIN)),
    ),
  };
}

/** Floating Chat's position and size, remembered on this device. */
function useFloatingBox() {
  const [stored, setStored] = useState<FloatingBox | undefined>(() => {
    try {
      const value = JSON.parse(window.localStorage.getItem(FLOAT_KEY) ?? "null");
      return value && ["right", "bottom", "width", "height"].every((k) => Number.isFinite(value[k]))
        ? value
        : undefined;
    } catch {
      return undefined;
    }
  });
  const box = clampFloatingBox(stored ?? defaultFloatingBox());
  const boxRef = useRef(box);
  useLayoutEffect(() => {
    boxRef.current = box;
  });
  const save = useCallback((next: FloatingBox) => {
    const value = clampFloatingBox(next);
    setStored(value);
    try {
      window.localStorage.setItem(FLOAT_KEY, JSON.stringify(value));
    } catch {
      // Storage disabled: the box lasts until reload.
    }
  }, []);
  /** Starts a drag; `apply` maps the pointer offset onto the starting box. */
  const drag = useCallback(
    (
      event: ReactPointerEvent,
      apply: (start: FloatingBox, dx: number, dy: number) => FloatingBox,
    ) => {
      if (event.button !== 0) return;
      event.preventDefault();
      const target = event.currentTarget as HTMLElement;
      target.setPointerCapture?.(event.pointerId);
      const start = boxRef.current;
      const x = event.clientX;
      const y = event.clientY;
      const move = (moveEvent: PointerEvent) =>
        save(apply(start, moveEvent.clientX - x, moveEvent.clientY - y));
      const up = () => {
        target.removeEventListener("pointermove", move);
        target.removeEventListener("pointerup", up);
        target.removeEventListener("pointercancel", up);
      };
      target.addEventListener("pointermove", move);
      target.addEventListener("pointerup", up);
      target.addEventListener("pointercancel", up);
    },
    [save],
  );
  const move = useCallback(
    (event: ReactPointerEvent) =>
      drag(event, (start, dx, dy) => ({
        ...start,
        right: start.right - dx,
        bottom: start.bottom - dy,
      })),
    [drag],
  );
  // The card is anchored bottom-right, so the top-left corner resizes it.
  const resize = useCallback(
    (event: ReactPointerEvent) =>
      drag(event, (start, dx, dy) => ({
        ...start,
        width: start.width - dx,
        height: start.height - dy,
      })),
    [drag],
  );
  const nudge = useCallback(
    (dx: number, dy: number) => {
      const start = boxRef.current;
      save({ ...start, right: start.right - dx, bottom: start.bottom - dy });
    },
    [save],
  );
  return { box, move, resize, nudge };
}

/** Drag to move the floating card; arrow keys move it too. */
function FloatingMoveHandle({
  onMove,
  onNudge,
}: {
  onMove: (event: ReactPointerEvent) => void;
  onNudge: (dx: number, dy: number) => void;
}) {
  const ui = useUiTranslations();
  return (
    <button
      type="button"
      className="chat-mode-float-move"
      aria-label={ui("Move chat")}
      title={ui("Drag to move")}
      onPointerDown={onMove}
      onKeyDown={(event) => {
        const step = {
          ArrowLeft: [-32, 0],
          ArrowRight: [32, 0],
          ArrowUp: [0, -32],
          ArrowDown: [0, 32],
        }[event.key];
        if (!step) return;
        event.preventDefault();
        onNudge(step[0], step[1]);
      }}
    >
      <GripHorizontal className="size-3.5" />
    </button>
  );
}

/** Top-left corner of the floating card: drag to resize. */
function FloatingResizeHandle({ onResize }: { onResize: (event: ReactPointerEvent) => void }) {
  const ui = useUiTranslations();
  return (
    <span
      className="chat-mode-float-resize"
      aria-hidden="true"
      title={ui("Drag to resize")}
      onPointerDown={onResize}
    />
  );
}
