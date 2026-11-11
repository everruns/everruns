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
  type CSSProperties,
  type ReactNode,
} from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import dynamic from "next/dynamic";
import { Columns2, Maximize2 } from "lucide-react";
import { Sidebar, MobileSidebar } from "@/components/layout/sidebar";
import { useFeatureFlags, useFeatureFlagsState } from "@/providers/feature-flags-provider";
import { ChatSwitcher } from "@/components/chat/chat-switcher";
import { useLandingRedirect } from "./landing";
import { useLayoutPreference, type AppLayout } from "@/hooks/use-layout-preference";
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

// Chat-first is the default layout (knowledge/ui/chat-mode.md): Main chat on the
// left, the page on the right, navigation in a rail on the right edge. Classic
// keeps the left sidebar and opens Chat full screen. The layout is a personal
// preference; the URL alone says which page is shown, so browser back and
// forward step through pages and threads.
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
  const conversationVisible = fullChat || beside;
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
  const { width: chatWidth, setWidth: setChatWidth, reset: resetChatWidth } = useChatWidth();
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
  // A remembered side chat from another organization or a deleted one falls
  // back to Main chat inside the workspace.
  const selectedChatId = activeChatId ?? mainChatId;
  const chatHref = useCallback(
    (chatId?: string) => (chatId && chatId !== mainChatId ? `/chats/${chatId}` : "/chats"),
    [mainChatId],
  );
  const showFullScreen = useCallback(() => {
    router.push(chatHref(selectedChatId));
  }, [router, chatHref, selectedChatId]);
  const showBesidePage = useCallback(() => {
    router.push(lastPage.current);
  }, [router]);
  const openBeside = useCallback(
    (chatId?: string) => {
      setActiveChatId(chatId === mainChatId ? undefined : chatId);
      if (chatRoute) router.push(lastPage.current);
    },
    [setActiveChatId, mainChatId, chatRoute, router],
  );
  const openThreadBeside = useCallback(
    (threadId: string) => router.push(`/threads/${threadId}`),
    [router],
  );
  const setOpen = useCallback(
    (value: boolean) => {
      if (value && chatFirst) {
        if (chatRoute) showBesidePage();
      } else if (value) router.push(chatHref(selectedChatId));
      else if (chatRoute) showBesidePage();
      else requestAnimationFrame(() => pageRef.current?.focus());
    },
    [chatFirst, chatRoute, showBesidePage, router, chatHref, selectedChatId],
  );

  useEffect(() => {
    // Cmd/Ctrl-J: chat-first focuses the composer; classic opens or closes
    // full-screen Chat. Cmd/Ctrl-Shift-J switches full-screen Chat and the page.
    const shortcut = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "j") return;
      event.preventDefault();
      const focusComposer = () =>
        document
          .querySelector<HTMLElement>(
            '.chat-mode-conversation textarea, .chat-mode-conversation [contenteditable="true"]',
          )
          ?.focus();
      if (event.shiftKey || !chatFirst) {
        if (chatRoute) showBesidePage();
        else showFullScreen();
      } else focusComposer();
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [chatFirst, chatRoute, showBesidePage, showFullScreen]);

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

  const mode = chatFirst ? (fullChat ? "chat" : "beside") : fullChat ? "classic-chat" : "classic";
  return (
    <ChatModeContext.Provider
      value={{
        pageContainer: pageRef,
        pageContainerElement: pageElement,
        open: beside,
        setOpen,
        page,
        registerPage,
        pageAdapter,
        pageViewAdapter: viewAdapter,
        threadId: selectedChatId,
        openBeside: chatFirst ? openBeside : undefined,
        showFullScreen,
        showBesidePage: chatFirst ? showBesidePage : undefined,
        openThreadBeside: chatFirst ? openThreadBeside : undefined,
        actions,
        recordAction,
      }}
    >
      <div
        className="chat-mode-shell min-h-0 flex-1"
        data-layout={mode}
        data-nav={navExpanded ? "expanded" : "rail"}
        style={{ "--chat-width": `${chatWidth}px` } as CSSProperties}
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
          hidden={!visited && !conversationVisible}
          inert={!conversationVisible}
          aria-hidden={!conversationVisible}
        >
          {(visited || conversationVisible) && thread ? (
            <ChatWorkspace
              activeChatId={activeChatId}
              onActiveChatChange={setActiveChatId}
              visible={conversationVisible}
              pageAware={beside}
              extraActions={
                beside ? (
                  <>
                    <ChatSwitcher
                      mainChatId={thread.id}
                      activeChatId={selectedChatId ?? thread.id}
                      onSelectChat={(id) => setActiveChatId(id === thread.id ? undefined : id)}
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
                  </>
                ) : fullChat ? (
                  <Button
                    variant="outline"
                    size="sm"
                    title={ui("Back to page (⌘⇧J)")}
                    onClick={showBesidePage}
                  >
                    <Columns2 className="size-3.5" />{" "}
                    {chatFirst ? ui("Show beside page") : ui("Back to page")}
                  </Button>
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
        {beside && (
          <ChatSplitter
            width={chatWidth}
            onWidthChange={setChatWidth}
            onReset={resetChatWidth}
            onCollapsePage={showFullScreen}
          />
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
const defaultChatWidth = () =>
  typeof window === "undefined" ? 560 : Math.round(Math.min(600, window.innerWidth * 0.4));

/** Chat pane width, remembered on this device and kept within both panes' minimums. */
function useChatWidth() {
  const [stored, setStored] = useState<number | undefined>(() => {
    try {
      const value = Number(window.localStorage.getItem(CHAT_WIDTH_KEY));
      return Number.isFinite(value) && value > 0 ? value : undefined;
    } catch {
      return undefined;
    }
  });
  const width = clampChatWidth(stored ?? defaultChatWidth());
  const setWidth = useCallback((value: number) => {
    const next = clampChatWidth(value);
    setStored(next);
    try {
      window.localStorage.setItem(CHAT_WIDTH_KEY, String(next));
    } catch {
      // Storage disabled: the width lasts until reload.
    }
  }, []);
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
) {
  return Math.round(Math.max(CHAT_MIN, Math.min(value, viewport - PAGE_MIN - 56)));
}

/**
 * Invisible divider between Chat and the page: a line appears on hover or
 * focus. Drag or use the arrow keys to resize; double-click resets. Dragging
 * the page narrower than half its minimum switches to full-screen Chat.
 */
function ChatSplitter({
  width,
  onWidthChange,
  onReset,
  onCollapsePage,
}: {
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
          onWidthChange(width + (event.key === "ArrowRight" ? 32 : -32));
        }
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        const target = event.currentTarget;
        target.setPointerCapture?.(event.pointerId);
        const left = target.parentElement?.getBoundingClientRect().left ?? 0;
        setDragging(true);
        const move = (moveEvent: PointerEvent) => onWidthChange(moveEvent.clientX - left);
        const up = (upEvent: PointerEvent) => {
          setDragging(false);
          target.removeEventListener("pointermove", move);
          target.removeEventListener("pointerup", up);
          target.removeEventListener("pointercancel", up);
          if (window.innerWidth - upEvent.clientX < PAGE_MIN / 2) onCollapsePage();
        };
        target.addEventListener("pointermove", move);
        target.addEventListener("pointerup", up);
        target.addEventListener("pointercancel", up);
      }}
    />
  );
}
