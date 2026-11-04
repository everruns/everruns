import { fireEvent, render, screen } from "@testing-library/react";
import { ChatWorkspace } from "@/components/chat/chat-workspace";
import { useChatThreads } from "@/hooks/use-chat-threads";
import type { Session, SessionTask } from "@/lib/api/types";

jest.mock("@/hooks/use-turn-work-report", () => ({
  useTurnWorkReport: () => null,
  useLiveWorkReport: () => null,
}));
jest.mock("@/components/chat/work-headline", () => ({
  WorkHeadline: ({ fallback }: { fallback: unknown }) => fallback,
}));

const mockTaskSubscription = jest.fn();
let mockSessions: Record<string, Partial<Session>> = {};
let mockBackgroundWork = true;
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
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => mockBackgroundWork,
}));
jest.mock("@/hooks/use-sessions", () => ({
  useSession: (id: string) => ({
    data: id
      ? { id, source: "chat", agent_id: "agent_platform", tags: ["chat"], ...mockSessions[id] }
      : undefined,
  }),
}));
jest.mock("@/hooks/use-chat-threads", () => ({ useChatThreads: jest.fn() }));
jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTasks: (id: string) => (
    mockTaskSubscription(id),
    {
      data: [
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
      ],
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
    <div>{threadMode ? "New thread draft" : "New subchat draft"}</div>
  ),
}));
jest.mock("@/components/chat/chat-thread-work-detail", () => ({
  ChatThreadWorkDetail: ({ task }: { task: SessionTask }) => <div>Work detail: {task.id}</div>,
}));

beforeEach(() => {
  mockBackgroundWork = true;
  mockSessions = {};
  mockTaskSubscription.mockClear();
  mockPath = "/chats";
  mockTaskId = null;
  mockPush.mockClear();
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: () => ({ matches: false, addEventListener: jest.fn(), removeEventListener: jest.fn() }),
  });
  jest.mocked(useChatThreads).mockReturnValue({
    threads: [
      {
        id: "side",
        title: "Investigate latency",
        status: "idle",
        tags: ["chat"],
        updated_at: "2026-10-03T00:00:00Z",
      } as Session,
    ],
    total: 21,
    isLoading: false,
    isRead: true,
    error: null,
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
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  expect(matchMedia).toHaveBeenCalledWith("(max-width: 1279px)");
  expect(screen.getByRole("dialog")).toHaveClass("w-full", "max-w-none");
  expect(screen.queryByRole("button", { name: "Expand thread panel" })).not.toBeInTheDocument();
});

it("lists independent subchats separately from background work", () => {
  render(<ChatWorkspace />);
  expect(screen.getByTestId("conversation-permanent")).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Subchats" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  // The conversation column clips and the threads pane stacks above it, so a
  // thread row cannot paint across the transcript.
  expect(screen.getByTestId("conversation-permanent").parentElement).toHaveClass(
    "overflow-hidden",
    "isolate",
  );
  expect(screen.getByRole("region", { name: "Subchats" })).toHaveClass(
    "overflow-hidden",
    "z-10",
    "bg-background",
  );
  expect(screen.getAllByRole("link", { name: "New subchat" })[0]).toHaveAttribute(
    "href",
    "/chats/new",
  );
  expect(screen.getByRole("link", { name: /Investigate latency/ })).toHaveAttribute(
    "href",
    "/chats/side",
  );
  expect(screen.queryByText("Needs you")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Work" }));
  expect(screen.getByText("Needs you")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Approve deployment/ })).toHaveAttribute(
    "href",
    "/chats/permanent?task=task_owned",
  );
  // A coordinator thread is one row, grouped by its latest assignment. It keeps
  // the name it started with; the current assignment leads the preview.
  expect(screen.getByText("Ready for review")).toBeInTheDocument();
  const threadRow = screen.getByRole("link", { name: /Draft the plan/ });
  expect(threadRow).toHaveAttribute("href", "/chats/thread_child");
  expect(threadRow).toHaveTextContent("Fix the login bug · Opened the PR");
  expect(screen.getAllByRole("link", { name: /Fix the login bug/ })).toHaveLength(1);
  fireEvent.click(screen.getByRole("tab", { name: "Subchats" }));
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(useChatThreads).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 20 }));
  fireEvent.change(screen.getByRole("textbox", { name: "Search threads" }), {
    target: { value: "latency" },
  });
  expect(useChatThreads).toHaveBeenLastCalledWith(
    expect.objectContaining({ search: "latency", offset: 0 }),
  );
});
it("replaces the main conversation when selecting an independent subchat", () => {
  const { rerender } = render(<ChatWorkspace />);
  mockPath = "/chats/side";
  rerender(<ChatWorkspace />);
  expect(screen.queryByTestId("conversation-permanent")).not.toBeInTheDocument();
  expect(screen.getByTestId("conversation-side")).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Subchats" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  fireEvent.click(screen.getByRole("button", { name: "Close subchats" }));
  expect(mockPush).not.toHaveBeenCalled();
});
it("opens a fresh draft as the only conversation", () => {
  mockPath = "/chats/new";
  render(<ChatWorkspace />);
  expect(screen.queryByTestId("conversation-permanent")).not.toBeInTheDocument();
  expect(screen.getByText("New subchat draft")).toBeInTheDocument();
});
it("keeps subchats available with background work disabled", () => {
  mockBackgroundWork = false;
  render(<ChatWorkspace />);
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  expect(screen.getByRole("link", { name: /Investigate latency/ })).toBeInTheDocument();
  expect(screen.queryByRole("tab", { name: "Work" })).not.toBeInTheDocument();
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

it("scopes Work to the selected subchat rather than permanent Chat", () => {
  mockPath = "/chats/side";
  render(<ChatWorkspace />);
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  fireEvent.click(screen.getByRole("tab", { name: "Work" }));
  expect(mockTaskSubscription).toHaveBeenLastCalledWith("side");
  expect(screen.getByRole("link", { name: /Approve deployment/ })).toHaveAttribute(
    "href",
    "/chats/side?task=task_owned",
  );
});
it("uses a delegated child's validated personal Chat parent for Work", () => {
  mockPath = "/chats/worker";
  mockSessions.worker = { parent_session_id: "side", agent_id: "agent_worker" };
  const onActiveChatChange = jest.fn();
  render(<ChatWorkspace onActiveChatChange={onActiveChatChange} />);
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  fireEvent.click(screen.getByRole("tab", { name: "Work" }));
  expect(mockTaskSubscription).toHaveBeenLastCalledWith("side");
  expect(onActiveChatChange).not.toHaveBeenCalled();
});
it("does not turn arbitrary parent sessions into a Chat workspace", () => {
  mockPath = "/chats/worker";
  mockSessions.worker = { parent_session_id: "unrelated", agent_id: "agent_worker" };
  mockSessions.unrelated = { source: "playground", tags: [], agent_id: "agent_worker" };
  render(<ChatWorkspace />);
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  fireEvent.click(screen.getByRole("tab", { name: "Work" }));
  expect(mockTaskSubscription).toHaveBeenLastCalledWith(undefined);
});
it("closes a drawer portal when page-side Chat is hidden", () => {
  const { rerender } = render(<ChatWorkspace pageAware visible />);
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  rerender(<ChatWorkspace pageAware visible={false} />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("supports arrow-key navigation between Subchats and Work", () => {
  render(<ChatWorkspace />);
  fireEvent.click(screen.getByRole("button", { name: "Subchats" }));
  const subchats = screen.getByRole("tab", { name: "Subchats" });
  subchats.focus();
  fireEvent.keyDown(subchats, { key: "ArrowRight" });
  const work = screen.getByRole("tab", { name: "Work" });
  expect(work).toHaveFocus();
  expect(work).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tabpanel")).toHaveAccessibleName("Work");
  fireEvent.keyDown(work, { key: "Home" });
  expect(subchats).toHaveFocus();
  expect(subchats).toHaveAttribute("aria-selected", "true");
});
