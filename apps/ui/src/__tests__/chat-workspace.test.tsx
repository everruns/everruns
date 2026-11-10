import { fireEvent, render, screen, within } from "@testing-library/react";
import { ChatWorkspace } from "@/components/chat/chat-workspace";
import { ChatModeContext, type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { Session, SessionTask } from "@/lib/api/types";

jest.mock("@/hooks/use-turn-work-report", () => ({
  useTurnWorkReport: () => null,
  useLiveWorkReport: () => null,
}));
jest.mock("@/components/chat/work-headline", () => ({
  WorkHeadline: ({ fallback }: { fallback: unknown }) => fallback,
}));

const mockTaskSubscription = jest.fn();
const INITIAL_TASKS = [
  {
    id: "task_owned",
    state: "awaiting_input",
    display_name: "Approve deployment",
    input_request: { prompt: "May I deploy?" },
    updated_at: "2026-10-03T00:00:00Z",
  },
  {
    id: "task_first",
    kind: "assignment",
    state: "succeeded",
    state_detail: "resolved",
    display_name: "Draft the plan",
    links: { child_session_id: "thread_child" },
    created_at: "2026-10-02T00:00:00Z",
    updated_at: "2026-10-02T00:00:00Z",
  },
  {
    id: "task_followup",
    kind: "assignment",
    state: "succeeded",
    display_name: "Fix the login bug",
    summary: "Opened the PR",
    links: { child_session_id: "thread_child" },
    created_at: "2026-10-03T00:00:00Z",
    updated_at: "2026-10-03T00:00:00Z",
  },
] as SessionTask[];
let mockTasks: SessionTask[] = INITIAL_TASKS;
let mockSessions: Record<string, Partial<Session>> = {};
let mockPath = "/chats";
let mockTaskId: string | null = null;
const mockPush = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPath,
  useSearchParams: () => ({ get: () => mockTaskId }),
  useRouter: () => ({ push: mockPush }),
}));
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    prefetch: _prefetch,
    ...props
  }: React.AnchorHTMLAttributes<HTMLAnchorElement> & { prefetch?: boolean }) => (
    <a {...props}>{children}</a>
  ),
}));
jest.mock("@/hooks/use-platform-chat-thread", () => ({
  usePlatformChatThread: () => ({ thread: { id: "permanent", agent_id: "agent_platform" } }),
}));
jest.mock("@/hooks/use-sessions", () => ({
  useSession: (id: string) => ({
    data: id
      ? { id, source: "chat", agent_id: "agent_platform", tags: ["chat"], ...mockSessions[id] }
      : undefined,
  }),
}));
jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTasks: (id: string) => (
    mockTaskSubscription(id),
    {
      data: mockTasks,
    }
  ),
}));
jest.mock("@/components/chat/chat-thread-view", () => ({
  ChatThreadView: ({
    threadId,
    extraActions,
  }: {
    threadId: string;
    extraActions?: React.ReactNode;
  }) => (
    <article data-testid={`conversation-${threadId}`}>
      {threadId}
      {extraActions}
    </article>
  ),
}));
jest.mock("@/app/(main)/chats/new/new-chat-page-client", () => ({
  ChatDraft: ({ threadMode }: { threadMode?: boolean }) => (
    <div>{threadMode ? "New thread draft" : "New side chat draft"}</div>
  ),
}));
jest.mock("@/components/chat/chat-thread-work-detail", () => ({
  ChatThreadWorkDetail: ({ task }: { task: SessionTask }) => <div>Work detail: {task.id}</div>,
}));

beforeEach(() => {
  mockTasks = INITIAL_TASKS;
  mockSessions = {};
  mockTaskSubscription.mockClear();
  mockPath = "/chats";
  mockTaskId = null;
  mockPush.mockClear();
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: () => ({ matches: false, addEventListener: jest.fn(), removeEventListener: jest.fn() }),
  });
});
it("uses a full-width drawer when the sidebar leaves insufficient room for split panes", () => {
  const matchMedia = jest.fn(() => ({
    matches: true,
    addEventListener: jest.fn(),
    removeEventListener: jest.fn(),
  }));
  Object.defineProperty(window, "matchMedia", { configurable: true, value: matchMedia });
  render(<ChatWorkspace />);
  fireEvent.click(screen.getByRole("button", { name: /^Threads/ }));
  expect(matchMedia).toHaveBeenCalledWith("(max-width: 1279px)");
  expect(screen.getByRole("dialog")).toHaveClass("w-full", "max-w-none");
  expect(screen.queryByRole("button", { name: "Expand thread panel" })).not.toBeInTheDocument();
});

it("lists the chat's threads on the right, grouped by what they need", () => {
  render(<ChatWorkspace />);
  expect(screen.getByTestId("conversation-permanent")).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Threads" })).not.toBeInTheDocument();
  // Needs you and Ready for review are open; the button counts them.
  const toggle = screen.getByRole("button", { name: /^Threads/ });
  expect(toggle).toHaveTextContent("Threads· 2");
  fireEvent.click(toggle);
  // The conversation column clips and the threads pane stacks above it, so a
  // thread row cannot paint across the transcript.
  expect(screen.getByTestId("conversation-permanent").parentElement).toHaveClass(
    "overflow-hidden",
    "isolate",
  );
  const panel = within(screen.getByRole("region", { name: "Threads" }));
  expect(screen.getByRole("region", { name: "Threads" })).toHaveClass(
    "overflow-hidden",
    "z-10",
    "bg-background",
  );
  expect(panel.getByText(/Needs you/)).toBeInTheDocument();
  expect(panel.getByText(/Ready for review/)).toBeInTheDocument();
  // A coordinator thread is one row, grouped by its latest assignment. It keeps
  // the name it started with; the current assignment leads the preview.
  const threadRow = panel.getByRole("button", { name: /Draft the plan/ });
  expect(threadRow).toHaveTextContent("Fix the login bug · Opened the PR");
  expect(panel.getAllByRole("button", { name: /Fix the login bug/ })).toHaveLength(1);
  // Other background work keeps its detail view.
  fireEvent.click(panel.getByRole("button", { name: /Approve deployment/ }));
  expect(mockPush).toHaveBeenCalledWith("/chats/permanent?task=task_owned");
});

it("opens a thread in the right panel next to the chat, with a way back", () => {
  render(<ChatWorkspace />);
  fireEvent.click(screen.getByRole("button", { name: /^Threads/ }));
  fireEvent.click(screen.getByRole("button", { name: /Draft the plan/ }));
  expect(screen.getByTestId("conversation-permanent")).toBeInTheDocument();
  expect(screen.getByTestId("conversation-thread_child")).toBeInTheDocument();
  expect(mockPush).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Expand thread panel" }));
  expect(screen.getByTestId("conversation-permanent").parentElement).toHaveClass("hidden");
  fireEvent.click(screen.getByRole("button", { name: /^Threads$/ }));
  expect(screen.queryByTestId("conversation-thread_child")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Draft the plan/ })).toBeInTheDocument();
});

it("opens a thread in the page pane when Chat sits beside a page", () => {
  const openThreadBeside = jest.fn();
  render(
    <ChatModeContext.Provider value={{ openThreadBeside } as unknown as ChatModeState}>
      <ChatWorkspace pageAware visible />
    </ChatModeContext.Provider>,
  );
  fireEvent.click(screen.getByRole("button", { name: /^Threads/ }));
  fireEvent.click(screen.getByRole("button", { name: /Draft the plan/ }));
  expect(openThreadBeside).toHaveBeenCalledWith("thread_child");
  expect(screen.queryByTestId("conversation-thread_child")).not.toBeInTheDocument();
});

it("opens the panel by itself when the chat starts a new thread", () => {
  const { rerender } = render(<ChatWorkspace />);
  expect(screen.queryByRole("region", { name: "Threads" })).not.toBeInTheDocument();
  mockTasks = [
    ...INITIAL_TASKS,
    {
      id: "task_new",
      kind: "assignment",
      state: "running",
      display_name: "Research pricing",
      links: { child_session_id: "thread_new" },
      created_at: "2026-10-04T00:00:00Z",
      updated_at: "2026-10-04T00:00:00Z",
    } as SessionTask,
  ];
  rerender(<ChatWorkspace />);
  expect(screen.getByRole("region", { name: "Threads" })).toBeInTheDocument();
  expect(screen.getByText(/Working/)).toBeInTheDocument();
});

it("explains how threads start when the chat has none", () => {
  mockTasks = [];
  render(<ChatWorkspace />);
  fireEvent.click(screen.getByRole("button", { name: /^Threads/ }));
  expect(
    screen.getByText("No threads yet. Ask this chat for longer work and it starts one."),
  ).toBeInTheDocument();
});

it("replaces the main conversation when selecting a side chat", () => {
  const { rerender } = render(<ChatWorkspace />);
  mockPath = "/chats/side";
  rerender(<ChatWorkspace />);
  expect(screen.queryByTestId("conversation-permanent")).not.toBeInTheDocument();
  expect(screen.getByTestId("conversation-side")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /^Threads/ }));
  fireEvent.click(screen.getByRole("button", { name: "Close threads" }));
  expect(mockPush).not.toHaveBeenCalled();
  expect(screen.getByRole("link", { name: "New side chat" })).toHaveAttribute("href", "/chats/new");
});
it("opens a fresh draft as the only conversation", () => {
  mockPath = "/chats/new";
  render(<ChatWorkspace />);
  expect(screen.queryByTestId("conversation-permanent")).not.toBeInTheDocument();
  expect(screen.getByText("New side chat draft")).toBeInTheDocument();
});
it("opens only work from the permanent conversation snapshot", () => {
  mockTaskId = "task_owned";
  const { rerender } = render(<ChatWorkspace />);
  expect(screen.getByText("Work detail: task_owned")).toBeInTheDocument();
  mockTaskId = "task_other_user";
  rerender(<ChatWorkspace />);
  expect(screen.queryByText("Work detail: task_owned")).not.toBeInTheDocument();
  expect(screen.getByText("This work is no longer available in your Chat.")).toBeInTheDocument();
});

it("scopes threads to the selected side chat rather than Main chat", () => {
  mockPath = "/chats/side";
  render(<ChatWorkspace />);
  expect(mockTaskSubscription).toHaveBeenLastCalledWith("side");
  fireEvent.click(screen.getByRole("button", { name: /^Threads/ }));
  fireEvent.click(screen.getByRole("button", { name: /Approve deployment/ }));
  expect(mockPush).toHaveBeenCalledWith("/chats/side?task=task_owned");
});
it("uses a delegated child's validated personal Chat parent for threads", () => {
  mockPath = "/chats/worker";
  mockSessions.worker = { parent_session_id: "side", agent_id: "agent_worker" };
  const onActiveChatChange = jest.fn();
  render(<ChatWorkspace onActiveChatChange={onActiveChatChange} />);
  expect(mockTaskSubscription).toHaveBeenLastCalledWith("side");
  expect(onActiveChatChange).not.toHaveBeenCalled();
});
it("does not turn arbitrary parent sessions into a Chat workspace", () => {
  mockPath = "/chats/worker";
  mockSessions.worker = { parent_session_id: "unrelated", agent_id: "agent_worker" };
  mockSessions.unrelated = { source: "playground", tags: [], agent_id: "agent_worker" };
  render(<ChatWorkspace />);
  expect(mockTaskSubscription).toHaveBeenLastCalledWith(undefined);
});
it("closes a drawer portal when page-side Chat is hidden", () => {
  const { rerender } = render(<ChatWorkspace pageAware visible />);
  fireEvent.click(screen.getByRole("button", { name: /^Threads/ }));
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  rerender(<ChatWorkspace pageAware visible={false} />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
