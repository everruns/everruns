import { fireEvent, render as rtlRender, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";
import { ChatModeShell, clampChatWidth } from "@/components/chat-mode/chat-mode-shell";
import { defaultNavigationSections, type NavigationSection } from "@/lib/navigation";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";

let mockPathname = "/agents/agent_1";
let mockCanNavigate = false;
let mockOrg = "org_1";
let mockHasRole = true;
const mockSearch = jest.fn();
const mockPush = jest.fn();
const mockQuery = new URLSearchParams();
function render(ui: ReactNode) {
  const client = new QueryClient();
  return rtlRender(ui, {
    wrapper: ({ children }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname,
  useSearchParams: () => mockQuery,
  useRouter: () => ({ replace: jest.fn(), push: mockPush }),
}));
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    prefetch: _prefetch,
    ...props
  }: {
    children: ReactNode;
    prefetch?: boolean;
  }) => <a {...props}>{children}</a>,
}));
jest.mock("next/dynamic", () => ({
  __esModule: true,
  default: () =>
    function ChatStub({
      extraActions,
      pageAware,
    }: {
      extraActions?: ReactNode;
      pageAware?: boolean;
    }) {
      const [text, setText] = useState("");
      return (
        <div data-testid="workspace" data-page-aware={pageAware}>
          {extraActions}
          <input
            aria-label="Chat draft"
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
          <textarea aria-label="Existing composer" />
        </div>
      );
    },
}));
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => true,
  useFeatureFlags: () => ({}),
  useFeatureFlagsState: () => ({ flags: {}, isLoading: false }),
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: mockOrg }, hasRole: () => mockHasRole }),
}));
jest.mock("@/hooks/use-policies", () => ({ usePolicies: () => ({ can: () => false }) }));
jest.mock("@/hooks/use-navigation-policy", () => ({
  useNavigationPolicy: () => () => mockCanNavigate,
}));
jest.mock("@/hooks/use-command-palette", () => ({
  useCommandPalette: () => ({ setOpen: mockSearch }),
}));
jest.mock("@/hooks/use-platform-chat-thread", () => ({
  usePlatformChatThread: () => ({ thread: { id: "session_chat" } }),
}));
jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));
jest.mock("@/components/chat/chat-switcher", () => ({
  ChatSwitcher: () => <button>Main chat</button>,
}));
jest.mock("@/components/layout/sidebar", () => ({
  Sidebar: ({ config }: { config?: { navigation?: unknown[] } }) => (
    <div>{config?.navigation ? "Full navigation" : "Classic sidebar"}</div>
  ),
  MobileSidebar: () => null,
}));
let mockLayout: "chat_first" | "classic" = "chat_first";
jest.mock("@/hooks/use-layout-preference", () => ({
  useLayoutPreference: () => ({ layout: mockLayout, ready: true, setLayout: jest.fn() }),
}));
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => ({
    user: { name: "Test User", email: "test@example.com" },
    requiresAuth: true,
    logout: jest.fn(),
    logoutPending: false,
  }),
}));
jest.mock("@/components/layout/sidebar-user-menu", () => ({
  SidebarUserMenu: ({ compact }: { compact?: boolean }) =>
    compact ? <button>Profile menu</button> : null,
}));
jest.mock("@/components/layout/sidebar-navigation", () => ({
  OverviewCountBadge: () => <span>3</span>,
}));

function Page() {
  const [count, setCount] = useState(0);
  return <button onClick={() => setCount(count + 1)}>Page edits {count}</button>;
}

function ModeProbe() {
  const mode = useChatMode();
  return (
    <>
      <output aria-label="Active chat">{mode?.threadId}</output>
      <button onClick={() => mode?.openBeside?.("side_1")}>Open side chat</button>
      <button onClick={() => mode?.openThreadBeside?.("thread_1")}>Open thread</button>
      <button onClick={() => mode?.setOpen(true)}>Open chat</button>
    </>
  );
}

function shell(children: ReactNode = <Page />, navigation?: NavigationSection[]) {
  return <ChatModeShell navigation={navigation}>{children}</ChatModeShell>;
}

beforeEach(() => {
  mockPathname = "/agents/agent_1";
  mockCanNavigate = false;
  mockOrg = "org_1";
  mockHasRole = true;
  mockLayout = "chat_first";
  window.localStorage.clear();
  window.sessionStorage.clear();
  jest.clearAllMocks();
});

describe("chat-first layout", () => {
  it("shows Main chat beside the page, with navigation in a rail on the right", () => {
    render(shell());
    const chat = screen.getByRole("region", { name: "Platform Chat" });
    expect(chat).not.toHaveAttribute("inert");
    expect(screen.getByTestId("workspace")).toHaveAttribute("data-page-aware", "true");
    expect(screen.getByText("Page edits 0")).toBeInTheDocument();
    expect(screen.getByRole("separator", { name: "Resize chat" })).toBeInTheDocument();
    const rail = screen.getByRole("navigation", { name: "Navigation" });
    expect(rail).toContainElement(screen.getByRole("link", { name: "Agents" }));
    expect(rail).toContainElement(screen.getByRole("button", { name: "Profile menu" }));
    expect(screen.queryByText("Classic sidebar")).not.toBeInTheDocument();
    expect(chat.closest("[data-layout]")).toHaveAttribute("data-layout", "beside");
  });

  it("keeps unavailable pages and the Chat section out of the rail", () => {
    mockHasRole = false;
    const icon = defaultNavigationSections[0].items[0].icon;
    const navigation: NavigationSection[] = [
      { id: "chats", items: [{ name: "Main chat", href: "/chats", icon }] },
      {
        label: "Pages",
        items: [
          { name: "Allowed page", href: "/allowed", icon },
          { name: "Policy restricted", href: "/restricted", icon, policy: "secret.view" },
          { name: "Owner only", href: "/owner", icon, minimumRole: "owner" as const },
          { name: "Disabled feature", href: "/voice", icon, flag: "voice" as const },
        ],
      },
    ];
    render(shell(<Page />, navigation));
    expect(screen.getByRole("link", { name: "Allowed page" })).toBeInTheDocument();
    for (const name of ["Main chat", "Policy restricted", "Owner only", "Disabled feature"]) {
      expect(screen.queryByRole("link", { name })).not.toBeInTheDocument();
    }
  });

  it("folds Settings into the System menu", async () => {
    render(shell());
    expect(screen.queryByRole("link", { name: "Settings" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "System and settings" }));
    expect(await screen.findByRole("menuitem", { name: "Settings" })).toBeInTheDocument();
  });

  it("expands the rail into the full sidebar and remembers it", () => {
    const { unmount } = render(shell());
    fireEvent.click(screen.getByRole("button", { name: "Expand navigation" }));
    expect(screen.getByText("Full navigation")).toBeInTheDocument();
    expect(window.localStorage.getItem("everruns.nav-expanded")).toBe("1");
    unmount();
    render(shell());
    expect(screen.getByText("Full navigation")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Collapse navigation" }));
    expect(screen.getByRole("link", { name: "Agents" })).toBeInTheDocument();
  });

  it("resizes Chat with the splitter, remembers the width and resets on double-click", () => {
    render(shell());
    const splitter = screen.getByRole("separator", { name: "Resize chat" });
    const before = Number(splitter.getAttribute("aria-valuenow"));
    fireEvent.keyDown(splitter, { key: "ArrowRight" });
    const after = Number(splitter.getAttribute("aria-valuenow"));
    expect(after).toBe(before + 32);
    expect(window.localStorage.getItem("everruns.chat-width")).toBe(String(after));
    expect(splitter.closest("[data-layout]")).toHaveStyle(`--chat-width: ${after}px`);
    fireEvent.doubleClick(splitter);
    expect(Number(splitter.getAttribute("aria-valuenow"))).toBe(before);
    expect(window.localStorage.getItem("everruns.chat-width")).toBeNull();
  });

  it("never lets either pane shrink below its minimum", () => {
    expect(clampChatWidth(100, 1440)).toBe(360);
    expect(clampChatWidth(2000, 1440)).toBe(1440 - 480 - 56);
  });

  it("opens a thread as a page route so the back button returns", () => {
    render(shell(<ModeProbe />));
    fireEvent.click(screen.getByText("Open thread"));
    expect(mockPush).toHaveBeenLastCalledWith("/threads/thread_1");
  });

  it("switches the left pane to a side chat for the rest of the visit", () => {
    const { unmount } = render(shell(<ModeProbe />));
    expect(screen.getByLabelText("Active chat")).toHaveTextContent("session_chat");
    fireEvent.click(screen.getByText("Open side chat"));
    expect(screen.getByLabelText("Active chat")).toHaveTextContent("side_1");
    expect(mockPush).not.toHaveBeenCalled();
    unmount();
    render(shell(<ModeProbe />));
    expect(screen.getByLabelText("Active chat")).toHaveTextContent("side_1");
  });

  it("resets the chat shell on organization switch", () => {
    const { rerender } = render(shell());
    fireEvent.change(screen.getByLabelText("Chat draft"), { target: { value: "Private org one" } });
    mockOrg = "org_2";
    rerender(shell());
    expect(screen.getByLabelText("Chat draft")).toHaveValue("");
  });

  it("labels a collection of page forms with the page title instead of the last form", () => {
    function MultipleForms() {
      const mode = useChatMode();
      const register = mode?.registerPage;
      useEffect(() => {
        if (!register) return;
        const adapter = (binding: string) => ({
          binding,
          describe: () => ({ binding, revision: "1", dirty: false, writable: false }),
          read: () => {
            throw new Error("Unused in label test");
          },
          patch: () => {
            throw new Error("Read only");
          },
        });
        const stop = [
          register({ route: "/agents/agent_1", label: "Current Agent" }, undefined, 2),
          register({ route: "/agents/agent_1", label: "Name", form_name: "name" }, adapter("name")),
          register(
            { route: "/agents/agent_1", label: "Outbound allowlist", form_name: "network" },
            adapter("network"),
          ),
        ];
        return () => stop.forEach((dispose) => dispose());
      }, [register]);
      return <output aria-label="Page context label">{mode?.page.label}</output>;
    }
    render(shell(<MultipleForms />));
    expect(screen.getByLabelText("Page context label")).toHaveTextContent("Current Agent");
  });

  it("shows one full Chat without its nested route content", () => {
    mockPathname = "/chats/side";
    render(shell(<div>Nested route conversation</div>));
    expect(screen.getAllByTestId("workspace")).toHaveLength(1);
    expect(screen.getByTestId("workspace")).toHaveAttribute("data-page-aware", "false");
    expect(screen.queryByText("Nested route conversation")).not.toBeInTheDocument();
    expect(screen.getByTestId("workspace").closest("[data-layout]")).toHaveAttribute(
      "data-layout",
      "chat",
    );
    expect(screen.queryByRole("separator")).not.toBeInTheDocument();
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(screen.getByLabelText("Existing composer")).toHaveFocus();
  });

  it("keeps the conversation and its draft between full Chat and the page", () => {
    mockPathname = "/chats/session_chat";
    const { rerender } = render(shell());
    const workspace = screen.getByTestId("workspace");
    fireEvent.change(screen.getByLabelText("Chat draft"), {
      target: { value: "Draft across views" },
    });
    mockPathname = "/agents/agent_1";
    rerender(shell());
    expect(screen.getByTestId("workspace")).toBe(workspace);
    expect(workspace).toHaveAttribute("data-page-aware", "true");
    mockPathname = "/chats/session_chat";
    rerender(shell());
    expect(screen.getByTestId("workspace")).toBe(workspace);
    expect(screen.getByLabelText("Chat draft")).toHaveValue("Draft across views");
  });

  it("switches between Chat beside the page and full screen", () => {
    const { rerender } = render(shell());
    fireEvent.click(screen.getByRole("button", { name: "Full screen" }));
    expect(mockPush).toHaveBeenLastCalledWith("/chats");
    mockPathname = "/chats";
    rerender(shell());
    fireEvent.click(screen.getByRole("button", { name: /Show beside page/ }));
    expect(mockPush).toHaveBeenLastCalledWith("/agents/agent_1");
  });

  it("focuses the composer with Cmd/Ctrl-J and goes full screen with Cmd/Ctrl-Shift-J", () => {
    render(shell());
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(screen.getByLabelText("Existing composer")).toHaveFocus();
    expect(mockPush).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "J", ctrlKey: true, shiftKey: true });
    expect(mockPush).toHaveBeenLastCalledWith("/chats");
  });
});

describe("classic layout", () => {
  beforeEach(() => {
    mockLayout = "classic";
  });

  it("keeps the left sidebar and the page, with no Chat beside it", () => {
    render(shell(<ModeProbe />));
    expect(screen.getByText("Classic sidebar")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Platform Chat" })).not.toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "Navigation" })).not.toBeInTheDocument();
    expect(screen.queryByRole("separator")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("Open chat"));
    expect(mockPush).toHaveBeenLastCalledWith("/chats");
  });

  it("toggles full-screen Chat with Cmd/Ctrl-J", () => {
    const { rerender } = render(shell());
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(mockPush).toHaveBeenLastCalledWith("/chats");
    mockPathname = "/chats";
    rerender(shell());
    expect(screen.getByTestId("workspace")).toHaveAttribute("data-page-aware", "false");
    fireEvent.click(screen.getByRole("button", { name: /Back to page/ }));
    expect(mockPush).toHaveBeenLastCalledWith("/agents/agent_1");
  });

  it("shows Chat history as a page", () => {
    mockPathname = "/chats/history";
    render(shell(<p>History list</p>));
    expect(screen.getByText("History list")).toBeInTheDocument();
    expect(screen.queryByTestId("workspace")).not.toBeInTheDocument();
  });
});
