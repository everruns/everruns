import { fireEvent, render, screen } from "@testing-library/react";
import { ChatWorkspace } from "@/components/chat/chat-workspace";
import { useChatThreads } from "@/hooks/use-chat-threads";
import type { Session, SessionTask } from "@/lib/api/types";

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
  usePlatformChatThread: () => ({ thread: { id: "permanent" } }),
}));
jest.mock("@/hooks/use-chat-threads", () => ({ useChatThreads: jest.fn() }));
jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTasks: () => ({
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
  }),
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
    <div>{threadMode ? "New thread draft" : "Legacy draft"}</div>
  ),
}));
jest.mock("@/components/chat/chat-thread-work-detail", () => ({
  ChatThreadWorkDetail: ({ task }: { task: SessionTask }) => <div>Work detail: {task.id}</div>,
}));

beforeEach(() => {
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
  mockPath = "/chats/history";
  render(<ChatWorkspace />);
  expect(matchMedia).toHaveBeenCalledWith("(max-width: 1279px)");
  expect(screen.getByRole("dialog")).toHaveClass("w-full", "max-w-none");
  expect(screen.queryByRole("button", { name: "Expand thread panel" })).not.toBeInTheDocument();
});

it("starts in permanent Chat and opens creation and history inside Threads", () => {
  render(<ChatWorkspace />);
  expect(screen.getByTestId("conversation-permanent")).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Threads" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Threads" }));
  expect(screen.getByRole("link", { name: "New thread" })).toHaveAttribute("href", "/chats/new");
  expect(screen.getByRole("link", { name: /Investigate latency/ })).toHaveAttribute(
    "href",
    "/chats/side",
  );
  expect(screen.getByText("Needs you")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Approve deployment/ })).toHaveAttribute(
    "href",
    "/chats?task=task_owned",
  );
  // A coordinator thread is one row, grouped by its latest assignment. It keeps
  // the name it started with; the current assignment leads the preview.
  expect(screen.getByText("Ready for review")).toBeInTheDocument();
  const threadRow = screen.getByRole("link", { name: /Draft the plan/ });
  expect(threadRow).toHaveAttribute("href", "/chats/thread_child");
  expect(threadRow).toHaveTextContent("Fix the login bug · Opened the PR");
  expect(screen.getAllByRole("link", { name: /Fix the login bug/ })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(useChatThreads).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 20 }));
  fireEvent.change(screen.getByRole("textbox", { name: "Search threads" }), {
    target: { value: "latency" },
  });
  expect(useChatThreads).toHaveBeenLastCalledWith(
    expect.objectContaining({ search: "latency", offset: 0 }),
  );
});
it("preserves the permanent conversation while opening and expanding a separate thread", () => {
  const { rerender } = render(<ChatWorkspace />);
  const permanent = screen.getByTestId("conversation-permanent");
  mockPath = "/chats/side";
  rerender(<ChatWorkspace />);
  expect(screen.getByTestId("conversation-permanent")).toBe(permanent);
  expect(screen.getByTestId("conversation-side")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Expand thread panel" }));
  expect(screen.getByRole("button", { name: "Restore split view" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Close threads" }));
  expect(mockPush).toHaveBeenLastCalledWith("/chats");
});
it("opens bookmarked drafts beside permanent Chat", () => {
  mockPath = "/chats/new";
  render(<ChatWorkspace />);
  expect(screen.getByTestId("conversation-permanent")).toBeInTheDocument();
  expect(screen.getByText("New thread draft")).toBeInTheDocument();
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
