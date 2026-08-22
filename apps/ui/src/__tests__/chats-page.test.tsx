import { Suspense } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import ChatsPageClient from "@/app/(main)/chats/chats-page-client";
import ChatThreadPage from "@/app/(main)/chats/[threadId]/page";
import { useChatThreads } from "@/hooks/use-chat-threads";
import type { Session } from "@/lib/api/types";

const mockPinMutate = jest.fn();
const mockUnpinMutate = jest.fn();
const mockArchiveMutate = jest.fn();
const mockUnarchiveMutate = jest.fn();

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
  useAgents: () => ({ data: [{ id: "agent_1", name: "scout", display_name: "Scout" }] }),
  useHarnesses: () => ({ data: [{ id: "harness_1", name: "generic", display_name: "Generic" }] }),
  usePageTitle: jest.fn(),
}));

jest.mock("@/components/chat/new-chat-form", () => ({
  NewChatForm: ({ onStartingChange }: { onStartingChange?: (starting: boolean) => void }) => (
    <button type="button" onClick={() => onStartingChange?.(true)}>
      new-chat-form
    </button>
  ),
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
    mockUseChatThreads.mockReturnValue({ threads: [], isLoading: false, error: null });
  });

  it("renders the empty state without feature-flag configuration", () => {
    render(<ChatsPageClient />);

    expect(screen.getByText("No chats yet")).toBeInTheDocument();
    expect(mockUseChatThreads).toHaveBeenCalled();
  });

  it("offers a way to start a chat when there are no threads yet", () => {
    render(<ChatsPageClient />);

    expect(screen.getByText("No chats yet")).toBeInTheDocument();
    expect(screen.getByText("new-chat-form")).toBeInTheDocument();
  });

  it("lists threads, linking each to its thread route", () => {
    mockUseChatThreads.mockReturnValue({
      threads: [thread({ id: "sess_1" }), thread({ id: "sess_2", title: "Latency" })],
      isLoading: false,
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
    expect(mockUseChatThreads).toHaveBeenLastCalledWith({ includeArchived: false });

    fireEvent.click(screen.getByRole("button", { name: /Filter/ }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Show archived" }));

    expect(mockUseChatThreads).toHaveBeenLastCalledWith({ includeArchived: true });
  });

  it("keeps the starting form mounted when the new thread lands in the list", () => {
    const { rerender } = render(<ChatsPageClient />);
    fireEvent.click(screen.getByText("new-chat-form"));

    // The created thread arrives from the invalidated session list. Swapping the
    // form out here would unmount it mid-navigation and strand the user on /chats.
    mockUseChatThreads.mockReturnValue({
      threads: [thread({ id: "sess_1" })],
      isLoading: false,
      error: null,
    });
    rerender(<ChatsPageClient />);

    expect(screen.getByText("new-chat-form")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /Standup/ })).not.toBeInTheDocument();
  });
});

describe("Thread surface", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseChatThreads.mockReturnValue({ threads: [], isLoading: false, error: null });
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1" }),
      agent: { id: "agent_1", name: "scout", display_name: "Scout" },
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
    expect(screen.getByText("chat-panel:Scout")).toBeInTheDocument();
  });

  it("does not mount mutable chat controls for a recording", async () => {
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1", tags: ["recording"] }),
      agent: { id: "agent_1", name: "scout", display_name: "Scout" },
      agentId: "agent_1",
      sessionLoading: false,
    });

    await renderThread();

    expect(screen.queryByText("chat-panel:Scout")).not.toBeInTheDocument();
    expect(screen.getByText("Thread not found")).toBeInTheDocument();
  });

  it("shows the bound agent in the header and names it in the composer", async () => {
    await renderThread();

    expect(screen.getByRole("button", { name: "Rename thread" })).toHaveTextContent("Standup");
    expect(screen.getByRole("link", { name: "Scout" })).toHaveAttribute("href", "/agents/agent_1");
    expect(screen.getByText("chat-panel:Scout")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open session/ })).toHaveAttribute(
      "href",
      "/sessions/sess_1/transcript",
    );
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

    expect(screen.getByText("Generic")).toBeInTheDocument();
    expect(screen.queryByText("No agent bound")).not.toBeInTheDocument();
  });

  it("titles an untitled thread from the list preview", async () => {
    // The session endpoint carries no preview; the list does.
    mockUseChatThreads.mockReturnValue({
      threads: [thread({ id: "sess_1", title: null, preview: "what broke last night?" })],
      isLoading: false,
      error: null,
    });
    mockSessionContext.mockReturnValue({
      session: thread({ id: "sess_1", title: null }),
      agent: { id: "agent_1", name: "scout", display_name: "Scout" },
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
