import { fireEvent, render as rtlRender, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { ChatModeShell } from "@/components/chat-mode/chat-mode-shell";
import { defaultNavigationSections } from "@/lib/navigation";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";

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
  usePathname: () => "/agents/agent_1",
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
    function ChatStub() {
      const [text, setText] = useState("");
      return (
        <input
          aria-label="Chat draft"
          value={text}
          onChange={(event) => setText(event.target.value)}
        />
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
