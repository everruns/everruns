import { Suspense } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import ChatsPageClient from "@/app/(main)/chats/chats-page-client";
import ChatThreadPage from "@/app/(main)/chats/[threadId]/page";
import { ChatThreadView } from "@/components/chat/chat-thread-view";
import { useChatThreads } from "@/hooks/use-chat-threads";
import type { Session } from "@/lib/api/types";

const mockPinMutate = jest.fn();
const mockUnpinMutate = jest.fn();
const mockArchiveMutate = jest.fn();
const mockUnarchiveMutate = jest.fn();

jest.mock("@/components/chat/streamdown-message", () => ({
  StreamdownMessage: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  InlineStreamdownMessage: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
}));
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) => (
    <a {...props}>{children}</a>
  ),
}));

jest.mock("@/hooks/use-chat-threads", () => ({
  useChatThreads: jest.fn(),
}));

jest.mock("@/hooks/use-sessions", () => ({
  useUpdateSession: () => ({ mutate: jest.fn() }),
  usePinSession: () => ({ mutate: mockPinMutate, isPending: false }),
  useUnpinSession: () => ({ mutate: mockUnpinMutate, isPending: false }),
  useArchiveSession: () => ({ mutate: mockArchiveMutate, isPending: false }),
  useUnarchiveSession: () => ({ mutate: mockUnarchiveMutate, isPending: false }),
}));

jest.mock("@/hooks", () => ({
  useAgents: () => ({
    data: [{ id: "agent_1", name: "platform-chat", display_name: "Platform Chat" }],
  }),
  useHarnesses: () => ({ data: [{ id: "harness_1", name: "generic", display_name: "Generic" }] }),
  usePageTitle: jest.fn(),
}));

// The page now decides whether its empty state is "No side chats yet" or the
// no-intelligence message, so it reads intelligence status (org context this
// suite does not stub). Default to available; the no-intelligence branch has
// its own coverage.
const intelligenceStatus = { isLoading: false, available: true, canManage: true };
jest.mock("@/hooks/use-intelligence", () => ({
  useIntelligenceStatus: () => intelligenceStatus,
}));

jest.mock("@/components/chat/chat-panel", () => ({
  ChatPanel: (props: { replyToLabel?: string }) => <div>chat-panel:{props.replyToLabel}</div>,
}));

const mockSessionContext = jest.fn();
jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionProvider: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  useSessionContext: () => mockSessionContext(),
}));

const mockUseChatThreads = jest.mocked(useChatThreads);

function thread(overrides: Partial<Session> & { id: string }): Session {
  return {
    organization_id: "org_1",
    harness_id: "harness_1",
    agent_id: "agent_1",
    owner_principal_id: "principal_1",
    title: "Standup",
    tags: ["chat"],
    model_id: null,
    status: "idle",
    created_at: "2026-08-09T10:00:00Z",
    updated_at: "2026-08-09T10:00:00Z",
    started_at: null,
    finished_at: null,
    ...overrides,
  } as Session;
}

describe("Chats surface", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseChatThreads.mockReturnValue({
      threads: [],
      isLoading: false,
      isRead: true,
      error: null,
    });
    intelligenceStatus.isLoading = false;
    intelligenceStatus.available = true;
    intelligenceStatus.canManage = true;
  });

  it("offers a way to start a chat when there are no threads yet", () => {
    render(<ChatsPageClient />);

    expect(screen.getByText("No side chats yet")).toBeInTheDocument();
    expect(screen.getAllByRole("link", { name: "New chat" })[0]).toBeInTheDocument();
  });

  it("swaps the whole empty state for the no-intelligence message", () => {
    intelligenceStatus.available = false;

    render(<ChatsPageClient />);

    expect(screen.getByText("No intelligence available")).toBeInTheDocument();
    // One centred message replaces the ordinary empty state.
    expect(screen.queryByText("No side chats yet")).not.toBeInTheDocument();
  });

  it("lists threads, linking each to its thread route", () => {
    mockUseChatThreads.mockReturnValue({
      threads: [thread({ id: "sess_1" }), thread({ id: "sess_2", title: "Latency" })],
      isLoading: false,
      isRead: true,
      error: null,
    });

    render(<ChatsPageClient />);

    expect(screen.getByRole("link", { name: /Standup/ })).toHaveAttribute("href", "/chats/sess_1");
    expect(screen.getByRole("link", { name: /Latency/ })).toHaveAttribute("href", "/chats/sess_2");
  });

  it("pins and unpins chats from the list", () => {
    mockUseChatThreads.mockReturnValue({
      threads: [
        thread({ id: "sess_1" }),
        thread({ id: "sess_2", title: "Pinned", is_pinned: true }),
      ],
      isLoading: false,
      isRead: true,
      error: null,
    });

    render(<ChatsPageClient />);
    fireEvent.click(screen.getByRole("button", { name: "Pin chat" }));
    fireEvent.click(screen.getByRole("button", { name: "Unpin chat" }));

    expect(mockPinMutate).toHaveBeenCalledWith({ sessionId: "sess_1" });
    expect(mockUnpinMutate).toHaveBeenCalledWith({ sessionId: "sess_2" });
  });

  it("archives and unarchives chats from the list", () => {
    mockUseChatThreads.mockReturnValue({
      threads: [
        thread({ id: "sess_1" }),
        thread({ id: "sess_2", title: "Done", archived_at: "2026-08-09T11:00:00Z" }),
      ],
      isLoading: false,
      isRead: true,
      error: null,
    });

    render(<ChatsPageClient />);
    fireEvent.click(screen.getByRole("button", { name: "Archive chat" }));
    fireEvent.click(screen.getByRole("button", { name: "Unarchive chat" }));

    expect(mockArchiveMutate).toHaveBeenCalledWith({ sessionId: "sess_1" });
    expect(mockUnarchiveMutate).toHaveBeenCalledWith({ sessionId: "sess_2" });
  });

  it("hides archived chats until the filter asks for them", () => {
    render(<ChatsPageClient />);

    // The list starts narrowed; the toggle is what widens it, and the hook is
    // what carries that to the server.
    expect(mockUseChatThreads).toHaveBeenLastCalledWith(
      expect.objectContaining({ includeArchived: false }),
    );

    fireEvent.click(screen.getByRole("button", { name: /Filter/ }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Show archived" }));

    expect(mockUseChatThreads).toHaveBeenLastCalledWith(
      expect.objectContaining({ includeArchived: true }),
    );
  });
});

describe("Thread surface", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseChatThreads.mockReturnValue({
      threads: [],
      isLoading: false,
      isRead: true,
      error: null,
    });
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1" }),
      agent: { id: "agent_1", name: "platform-chat", display_name: "Platform Chat" },
      agentId: "agent_1",
      sessionLoading: false,
    });
  });

  // `use(params)` suspends until the route params settle, so each render is
  // flushed inside act() before the assertions look at the surface.
  async function renderThread() {
    await act(async () => {
      render(
        <Suspense fallback={null}>
          <ChatThreadPage params={Promise.resolve({ threadId: "sess_1" })} />
        </Suspense>,
      );
    });
  }

  it("loads the thread without feature-flag configuration", async () => {
    await renderThread();

    expect(mockSessionContext).toHaveBeenCalled();
    expect(screen.getByText("chat-panel:Platform Chat")).toBeInTheDocument();
  });

  it("resolves and reopens an adopted thread through the existing lifecycle", () => {
    const { rerender } = render(<ChatThreadView threadId="sess_1" threadMode />);
    fireEvent.click(screen.getByRole("button", { name: "Resolve thread" }));
    expect(mockArchiveMutate).toHaveBeenCalledWith({ sessionId: "sess_1" });
    expect(screen.queryByRole("button", { name: "Pin chat" })).not.toBeInTheDocument();
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1", archived_at: "2026-10-03T00:00:00Z" }),
      agent: { id: "agent_1", name: "platform-chat", display_name: "Platform Chat" },
      agentId: "agent_1",
      sessionLoading: false,
    });
    rerender(<ChatThreadView threadId="sess_1" threadMode />);
    expect(screen.getByText("Resolved")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reopen thread" }));
    expect(mockUnarchiveMutate).toHaveBeenCalledWith({ sessionId: "sess_1" });
  });

  it("shares permanent Chat independently of the selected side-pane route", async () => {
    const writeText = jest.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    mockSessionContext.mockReturnValue({
      session: thread({ id: "permanent", tags: ["chat", "platform-chat-starter"] }),
      agent: { id: "agent_1", name: "platform-chat", display_name: "Platform Chat" },
      agentId: "agent_1",
      sessionLoading: false,
    });
    render(<ChatThreadView threadId="permanent" />);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Share" })));
    expect(writeText).toHaveBeenCalledWith(`${window.location.origin}/chats`);
  });

  it("does not mount mutable chat controls for a recording", async () => {
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1", tags: ["recording"] }),
      agent: { id: "agent_1", name: "platform-chat", display_name: "Platform Chat" },
      agentId: "agent_1",
      sessionLoading: false,
    });

    await renderThread();

    expect(screen.queryByText("chat-panel:Platform Chat")).not.toBeInTheDocument();
    expect(screen.getByText("Thread not found")).toBeInTheDocument();
  });

  it("shows the bound agent in the header and names it in the composer", async () => {
    await renderThread();

    expect(screen.getByRole("button", { name: "Rename thread" })).toHaveTextContent("Standup");
    expect(screen.getByRole("link", { name: "Platform Chat" })).toHaveAttribute(
      "href",
      "/agents/agent_1",
    );
    expect(screen.getByText("chat-panel:Platform Chat")).toBeInTheDocument();
    const openSession = screen.getByRole("link", { name: /Open session/ });
    expect(openSession).toHaveAttribute("href", "/sessions/sess_1/transcript");
    // Toolbar siblings must share the sm control height (h-7), not a taller
    // hand-padded link.
    expect(openSession).toHaveClass("h-7");
    expect(screen.getByRole("button", { name: "Pin chat" })).toHaveClass("h-7");
    expect(screen.getByRole("button", { name: "Archive chat" })).toHaveClass("h-7");
    expect(screen.getByRole("button", { name: "Share" })).toHaveClass("h-7");
    fireEvent.click(screen.getByRole("button", { name: "Pin chat" }));
    expect(mockPinMutate).toHaveBeenCalledWith({ sessionId: "sess_1" });
  });

  it("names the harness for a thread bound to one instead of an agent", async () => {
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1", agent_id: null }),
      agent: undefined,
      agentId: undefined,
      sessionLoading: false,
    });

    await renderThread();

    expect(screen.getByText("Thread not found")).toBeInTheDocument();
    expect(screen.queryByText("No agent bound")).not.toBeInTheDocument();
  });

  it("titles an untitled thread from the list preview", async () => {
    // The session endpoint carries no preview; the list does.
    mockUseChatThreads.mockReturnValue({
      threads: [thread({ id: "sess_1", title: null, preview: "what broke last night?" })],
      isLoading: false,
      isRead: true,
      error: null,
    });
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1", title: null }),
      agent: { id: "agent_1", name: "platform-chat", display_name: "Platform Chat" },
      agentId: "agent_1",
      sessionLoading: false,
    });

    await renderThread();

    expect(screen.getByRole("button", { name: "Rename thread" })).toHaveTextContent(
      "what broke last night?",
    );
  });

  it("says so when the thread does not exist", async () => {
    mockSessionContext.mockReturnValue({
      session: undefined,
      agent: undefined,
      agentId: undefined,
      sessionLoading: false,
    });

    await renderThread();

    expect(screen.getByText("Thread not found")).toBeInTheDocument();
  });
});
