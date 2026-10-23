import { fireEvent, render, screen } from "@testing-library/react";
import { useState, type ReactNode } from "react";
import { ChatModeShell } from "@/components/chat-mode/chat-mode-shell";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";

let mockEnabled = false;
let mockOrg = "org_1";
const mockSearch = jest.fn();
const mockQuery = new URLSearchParams();
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
  useOrg: () => ({ currentOrg: { public_id: mockOrg }, hasRole: () => true }),
}));
jest.mock("@/hooks/use-policies", () => ({ usePolicies: () => ({ can: () => false }) }));
jest.mock("@/hooks/use-navigation-policy", () => ({ useNavigationPolicy: () => () => false }));
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
  mockOrg = "org_1";
  jest.clearAllMocks();
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
