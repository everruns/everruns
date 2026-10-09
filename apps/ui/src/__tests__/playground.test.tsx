import { Suspense } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import PlaygroundPage from "@/app/(main)/playground/page";
import PlaygroundChatPage from "@/app/(main)/playground/[sessionId]/page";
import { ChatThreadHeader } from "@/components/chat/chat-thread-header";
import { getSessionFacets, listSessions } from "@/lib/api/sessions";
import type { Event, Session } from "@/lib/api/types";

const mockSession: Session = {
  id: "session_test",
  source: "playground",
  title: "Support test",
  preview: "Hello",
  agent_id: "agent_support",
  harness_id: "harness_base",
  playground_user_id: "identity_customer",
  activity: "idle",
  updated_at: "2026-10-03T10:00:00Z",
  tags: [],
  organization_id: "org_test",
  owner_principal_id: "principal_test",
  model_id: null,
  status: "started",
  created_at: "2026-10-03T10:00:00Z",
  started_at: null,
  finished_at: null,
};
let mockChatEvents: Event[] = [];
const mockAgent = { id: "agent_support", name: "Support agent", status: "active" };
const mockMutation = { mutate: jest.fn(), isPending: false };

jest.mock("@/hooks/use-chat-mcp-servers", () => ({
  useChatMcpServers: () => ({ data: [], isLoading: false, error: null, refetch: jest.fn() }),
  useRemoveChatMcpServer: () => ({ mutate: jest.fn(), isPending: false, error: null }),
}));
jest.mock("next/navigation", () => ({ usePathname: () => "/playground/session_test" }));
jest.mock("@/lib/api/sessions", () => ({
  listSessions: jest.fn(),
  getSessionFacets: jest.fn(),
}));
jest.mock("@/hooks", () => ({
  useAgents: () => ({ data: [mockAgent] }),
  useHarnesses: () => ({ data: [] }),
  usePageTitle: jest.fn(),
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_test", name: "Test org" }, hasRole: () => true }),
}));
jest.mock("@/hooks/use-virtual-users", () => ({
  useVirtualUser: () => ({ data: { id: "identity_customer", name: "Alex", status: "active" } }),
}));
jest.mock("@/hooks/use-sessions", () => ({
  useUpdateSession: () => mockMutation,
  useArchiveSession: () => mockMutation,
  useUnarchiveSession: () => mockMutation,
  usePinSession: () => mockMutation,
  useUnpinSession: () => mockMutation,
}));
jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionProvider: ({ children }: { children: React.ReactNode }) => children,
  useSessionContext: () => ({
    session: mockSession,
    agent: mockAgent,
    chatEvents: mockChatEvents,
    getMessageText: () => "First user question",
  }),
}));
jest.mock("@/components/chat/chat-panel", () => ({ ChatPanel: () => <div>Live chat</div> }));
jest.mock("@/components/session/session-transcript", () => ({
  SessionTranscript: () => <div>Transcript</div>,
}));
jest.mock("@/components/session/session-workspace", () => ({
  SessionWorkspace: () => <div>Workspace files</div>,
}));
jest.mock("@/components/chat/streamdown-message", () => ({ InlineStreamdownMessage: () => null }));

function renderLibrary() {
  return render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <PlaygroundPage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  mockSession.title = "Support test";
  mockSession.preview = "Hello";
  mockSession.agent_id = "agent_support";
  mockChatEvents = [];
  jest
    .mocked(listSessions)
    .mockResolvedValue({ data: [mockSession], total: 25, offset: 0, limit: 20 });
  jest.mocked(getSessionFacets).mockResolvedValue({
    total: 12,
    by_activity: [],
    by_source: [],
    by_agent: [{ value: "agent_support", count: 4 }],
    active_now: 1,
    failed_today: 0,
    p95_duration_ms: 0,
    tokens_today: 0,
  });
});

test("the shared list opens from the row and links agent and user context", async () => {
  mockSession.effective_owner = {
    id: "principal_human",
    kind: "user",
    metadata: { name: "Mykola Chaliy" },
  };
  mockSession.event_count = 14;
  renderLibrary();
  expect(await screen.findByRole("link", { name: "Support test" })).toHaveAttribute(
    "href",
    "/playground/session_test",
  );
  expect(screen.queryByRole("link", { name: "Open chat Support test" })).not.toBeInTheDocument();
  expect(screen.getByText("Hello")).toBeInTheDocument();
  expect(screen.getByTitle("Started by Mykola Chaliy")).toHaveTextContent("MC");
  expect(screen.getByTitle("14 events")).toBeInTheDocument();
  expect(screen.getByText("12")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Support agent" })).toHaveAttribute(
    "href",
    "/agents/agent_support",
  );
  expect(screen.getByRole("link", { name: "Alex" })).toHaveAttribute(
    "href",
    "/virtual-users/identity_customer",
  );
  expect(screen.getByRole("link", { name: "New chat" })).toHaveAttribute("href", "/playground/new");
});

test("archived tabs and pagination preserve server-side Playground filtering", async () => {
  renderLibrary();
  await screen.findByRole("link", { name: "Support test" });
  fireEvent.click(screen.getByRole("tab", { name: "Archived" }));
  await waitFor(() =>
    expect(listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ source: "playground", archivedOnly: true, offset: 0 }),
    ),
  );
  fireEvent.click(await screen.findByRole("button", { name: "Next" }));
  await waitFor(() =>
    expect(listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ source: "playground", archivedOnly: true, offset: 20 }),
    ),
  );
});

test("agent filter and grouping controls the current page without a virtual-user select", async () => {
  renderLibrary();
  await screen.findByRole("link", { name: "Support test" });
  expect(screen.queryByText("Virtual user filter")).not.toBeInTheDocument();
  expect(screen.getByRole("radio", { name: "Day" })).toHaveAttribute("aria-checked", "true");
  fireEvent.click(screen.getByRole("button", { name: "Filter" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: /Support agent/ }));
  await waitFor(() =>
    expect(listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ source: "playground", agentId: "agent_support", offset: 0 }),
    ),
  );
  expect(screen.getByText("Agent is")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("radio", { name: "Agent" }));
  expect(screen.getByRole("group", { name: "Support agent" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("radio", { name: "None" }));
  expect(
    screen.queryByRole("group", { name: /Today|Yesterday|Earlier|Support agent/ }),
  ).not.toBeInTheDocument();
});

test("chats keep their agent link when the agent is absent from the active list", async () => {
  mockSession.agent_id = "agent_archived";
  renderLibrary();
  expect(await screen.findByRole("link", { name: "agent_archived" })).toHaveAttribute(
    "href",
    "/agents/agent_archived",
  );
  expect(screen.queryByText("Harness chat")).not.toBeInTheDocument();
});

test("chat breadcrumbs return to Playground and session/workspace inspection uses existing surfaces", async () => {
  const params = Promise.resolve({ sessionId: mockSession.id });
  await act(async () => {
    render(
      <Suspense>
        <PlaygroundChatPage params={params} />
      </Suspense>,
    );
  });
  expect(await screen.findByRole("link", { name: "Playground" })).toHaveAttribute(
    "href",
    "/playground",
  );
  expect(screen.getAllByRole("link", { name: "Support agent" })[0]).toHaveAttribute(
    "href",
    "/agents/agent_support",
  );
  expect(screen.getByRole("link", { name: "Alex" })).toHaveAttribute(
    "href",
    "/virtual-users/identity_customer",
  );
  expect(screen.getAllByRole("link", { name: "Open session" })[0]).toHaveAttribute(
    "href",
    "/sessions/session_test/transcript",
  );
  expect(screen.queryByRole("link", { name: "Trace" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Pin chat" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Workspace" }));
  expect(screen.getByText("Workspace files")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Chat" }));
  expect(screen.getByText("Live chat")).toBeInTheDocument();
});

test("the shared header retains personal Chat pinning", () => {
  render(<ChatThreadHeader session={mockSession} title="Personal chat" />);
  fireEvent.click(screen.getByRole("button", { name: "Pin chat" }));
  expect(mockMutation.mutate).toHaveBeenCalledWith({ sessionId: mockSession.id });
});

test("an untitled chat uses its first user input in the header and breadcrumb", async () => {
  mockSession.title = null;
  mockSession.preview = null;
  mockChatEvents = [
    {
      id: "event_input",
      type: "input.message",
      session_id: mockSession.id,
      ts: mockSession.created_at,
      context: {},
      data: { message: { role: "user", content: [{ type: "text", text: "First user question" }] } },
    },
  ];
  const params = Promise.resolve({ sessionId: mockSession.id });
  await act(async () => {
    render(
      <Suspense>
        <PlaygroundChatPage params={params} />
      </Suspense>,
    );
  });
  expect(screen.getByRole("button", { name: "Rename thread" })).toHaveTextContent(
    "First user question",
  );
  expect(screen.getByRole("navigation", { name: "Breadcrumb" })).toHaveTextContent(
    "First user question",
  );
});
