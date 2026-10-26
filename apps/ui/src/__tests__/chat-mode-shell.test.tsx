import { fireEvent, render as rtlRender, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";
import { ChatModeShell } from "@/components/chat-mode/chat-mode-shell";
import { defaultNavigationSections } from "@/lib/navigation";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";

let mockPathname = "/agents/agent_1";
let mockEnabled = false;
let mockCanNavigate = false;
let mockOrg = "org_1";
let mockHasRole = true;
const mockSearch = jest.fn();
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
  useFeatureFlag: () => mockEnabled,
  useFeatureFlags: () => ({}),
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
jest.mock("@/components/layout/sidebar", () => ({
  Sidebar: function SidebarStub() {
    const mode = useChatMode();
    return <button onClick={() => mode?.setOpen(true)}>Chat entry</button>;
  },
  MobileSidebar: () => null,
}));

function Page() {
  const [count, setCount] = useState(0);
  return <button onClick={() => setCount(count + 1)}>Page edits {count}</button>;
}
beforeEach(() => {
  mockPathname = "/agents/agent_1";
  mockEnabled = false;
  mockCanNavigate = false;
  mockOrg = "org_1";
  mockHasRole = true;
  jest.clearAllMocks();
});

it("keeps unavailable pages out of both the rail and the labeled menu", async () => {
  mockEnabled = true;
  mockHasRole = false;
  const icon = defaultNavigationSections[0].items[0].icon;
  const navigation = [
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
  render(
    <ChatModeShell navigation={navigation}>
      <Page />
    </ChatModeShell>,
  );
  fireEvent.click(screen.getByText("Chat entry"));
  expect(screen.getByRole("link", { name: "Allowed page" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Browse pages" }));
  await screen.findByRole("menu");
  expect(screen.getByRole("menuitem", { name: "Allowed page" })).toBeInTheDocument();
  for (const name of ["Policy restricted", "Owner only", "Disabled feature"]) {
    expect(screen.queryByRole("link", { name })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name })).not.toBeInTheDocument();
  }
});

it("keeps the existing shell when the organization has not opted in", () => {
  render(
    <ChatModeShell>
      <Page />
    </ChatModeShell>,
  );
  expect(screen.queryByRole("region", { name: "Platform Chat" })).not.toBeInTheDocument();
  expect(screen.getByText("Page edits 0")).toBeInTheDocument();
});

it("opens and closes without remounting the current page or losing the chat draft", () => {
  mockEnabled = true;
  render(
    <ChatModeShell>
      <Page />
    </ChatModeShell>,
  );
  fireEvent.click(screen.getByText("Page edits 0"));
  fireEvent.click(screen.getByText("Chat entry"));
  expect(screen.getByText("Chat entry").parentElement).toHaveAttribute("inert");
  expect(screen.getByText("Showing this page to the chat")).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Chat draft"), { target: { value: "Keep my draft" } });
  fireEvent.click(screen.getByText("Back to app"));
  expect(screen.getByText("Page edits 1")).toBeInTheDocument();
  fireEvent.keyDown(window, { key: "j", ctrlKey: true });
  expect(screen.getByLabelText("Chat draft")).toHaveValue("Keep my draft");
  expect(screen.getByText("Page edits 1")).toBeInTheDocument();
});

it("resets the chat shell on organization switch", () => {
  mockEnabled = true;
  const { rerender } = render(
    <ChatModeShell>
      <Page />
    </ChatModeShell>,
  );
  fireEvent.click(screen.getByText("Chat entry"));
  fireEvent.change(screen.getByLabelText("Chat draft"), { target: { value: "Private org one" } });
  mockOrg = "org_2";
  rerender(
    <ChatModeShell>
      <Page />
    </ChatModeShell>,
  );
  expect(screen.queryByLabelText("Chat draft")).not.toBeInTheDocument();
  fireEvent.click(screen.getByText("Chat entry"));
  expect(screen.getByLabelText("Chat draft")).toHaveValue("");
});

it("uses the host's navigation sections in the shared rail", () => {
  mockEnabled = true;
  mockCanNavigate = true;
  const navigation = defaultNavigationSections.map((section) => ({
    ...section,
    items: section.items
      .filter((item) => item.href === "/settings/organization")
      .map((item) => ({
        ...item,
        name: "Hosted settings",
      })),
  }));
  render(
    <ChatModeShell navigation={navigation}>
      <Page />
    </ChatModeShell>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Chat entry" }));
  expect(screen.getByRole("link", { name: "Hosted settings" })).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Agents" })).not.toBeInTheDocument();
});

it("labels a collection of page forms with the page title instead of the last form", () => {
  mockEnabled = true;
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
  render(
    <ChatModeShell>
      <MultipleForms />
    </ChatModeShell>,
  );
  expect(screen.getByLabelText("Page context label")).toHaveTextContent("Current Agent");
});

it("shows one full Chat without page-side Chat or its nested route content", () => {
  mockEnabled = true;
  mockPathname = "/chats/side";
  render(
    <ChatModeShell>
      <div>Nested route conversation</div>
    </ChatModeShell>,
  );
  expect(screen.getAllByTestId("workspace")).toHaveLength(1);
  expect(screen.getByTestId("workspace")).toHaveAttribute("data-page-aware", "false");
  expect(screen.queryByText("Nested route conversation")).not.toBeInTheDocument();
  expect(screen.queryByText("Showing this page to the chat")).not.toBeInTheDocument();
  expect(screen.getByTestId("workspace").closest("[data-chat-mode]")).toHaveAttribute(
    "data-chat-mode",
    "chat",
  );
  fireEvent.keyDown(window, { key: "j", ctrlKey: true });
  expect(screen.getByLabelText("Existing composer")).toHaveFocus();
});
it("preserves a conversation draft between full Chat and the page workspace", () => {
  mockEnabled = true;
  mockPathname = "/chats/session_chat";
  const { rerender } = render(
    <ChatModeShell>
      <Page />
    </ChatModeShell>,
  );
  const workspace = screen.getByTestId("workspace");
  fireEvent.change(screen.getByLabelText("Chat draft"), {
    target: { value: "Draft across views" },
  });
  mockPathname = "/agents/agent_1";
  rerender(
    <ChatModeShell>
      <Page />
    </ChatModeShell>,
  );
  fireEvent.keyDown(window, { key: "j", ctrlKey: true });
  expect(screen.getByTestId("workspace")).toBe(workspace);
  expect(screen.getByLabelText("Chat draft")).toHaveValue("Draft across views");
  expect(workspace).toHaveAttribute("data-page-aware", "true");
  mockPathname = "/chats/session_chat";
  rerender(
    <ChatModeShell>
      <Page />
    </ChatModeShell>,
  );
  expect(screen.getByTestId("workspace")).toBe(workspace);
  expect(workspace).toHaveAttribute("data-page-aware", "false");
  expect(screen.getByLabelText("Chat draft")).toHaveValue("Draft across views");
});
it("shows Chat history without another conversation pane", () => {
  mockEnabled = true;
  mockPathname = "/chats/history";
  render(
    <ChatModeShell>
      <p>History list</p>
    </ChatModeShell>,
  );
  expect(screen.getByText("History list")).toBeInTheDocument();
  expect(screen.queryByTestId("workspace")).not.toBeInTheDocument();
  fireEvent.keyDown(window, { key: "j", ctrlKey: true });
  expect(screen.queryByTestId("workspace")).not.toBeInTheDocument();
});
