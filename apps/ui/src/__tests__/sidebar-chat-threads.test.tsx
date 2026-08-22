import { fireEvent, render, screen } from "@testing-library/react";
import { SidebarChatThreads } from "@/components/layout/sidebar-chat-threads";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { SIDEBAR_THREAD_LIMIT } from "@/lib/chat-threads";
import type { Session } from "@/lib/api/types";

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

jest.mock("@/hooks/use-chat-threads", () => ({
  useChatThreads: jest.fn(),
}));

const mockUseChatThreads = jest.mocked(useChatThreads);

function thread(id: string, title: string): Session {
  return {
    id,
    organization_id: "org_1",
    harness_id: "harness_1",
    agent_id: "agent_1",
    owner_principal_id: "principal_1",
    title,
    tags: ["chat"],
    model_id: null,
    status: "idle",
    created_at: "2026-08-09T10:00:00Z",
    updated_at: "2026-08-09T10:00:00Z",
    started_at: null,
    finished_at: null,
  } as Session;
}

function setThreads(threads: Session[]) {
  mockUseChatThreads.mockReturnValue({ threads, isLoading: false, error: null });
}

describe("SidebarChatThreads", () => {
  beforeEach(() => jest.clearAllMocks());

  it("caps the list and offers the way out to all chats", () => {
    setThreads(Array.from({ length: 8 }, (_, index) => thread(`sess_${index}`, `Thread ${index}`)));

    render(<SidebarChatThreads pathname="/chats" />);

    expect(screen.getByText("Thread 0")).toBeInTheDocument();
    expect(screen.getByText(`Thread ${SIDEBAR_THREAD_LIMIT - 1}`)).toBeInTheDocument();
    expect(screen.queryByText(`Thread ${SIDEBAR_THREAD_LIMIT}`)).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "All chats" })).toHaveAttribute("href", "/chats");
    expect(screen.getByRole("link", { name: /New chat/ })).toHaveAttribute("href", "/chats/new");
  });

  it("keeps the way out to all chats even with no threads", () => {
    // /chats is the only place archived threads can be brought back into view,
    // so the link must not disappear with the list.
    setThreads([]);

    render(<SidebarChatThreads pathname="/chats" />);

    expect(screen.getByRole("link", { name: "All chats" })).toHaveAttribute("href", "/chats");
  });

  it("marks pinned threads", () => {
    mockUseChatThreads.mockReturnValue({
      threads: [{ ...thread("sess_pinned", "Pinned thread"), is_pinned: true }],
      isLoading: false,
      error: null,
    });

    render(<SidebarChatThreads pathname="/chats" />);

    expect(screen.getByLabelText("Pinned")).toBeInTheDocument();
  });

  it("holds the order steady while the pointer is inside the list", () => {
    const first = thread("sess_a", "Alpha");
    const second = thread("sess_b", "Beta");
    setThreads([first, second]);

    const { rerender, container } = render(<SidebarChatThreads pathname="/chats" />);
    const list = container.firstElementChild as HTMLElement;

    fireEvent.mouseEnter(list);
    // A turn lands on Beta and re-sorts the query result under the cursor.
    setThreads([second, first]);
    rerender(<SidebarChatThreads pathname="/chats" />);

    const titles = screen.getAllByRole("link").map((link) => link.textContent);
    expect(titles.slice(0, 2)).toEqual(["Alpha", "Beta"]);

    fireEvent.mouseLeave(list);
    rerender(<SidebarChatThreads pathname="/chats" />);

    const reordered = screen.getAllByRole("link").map((link) => link.textContent);
    expect(reordered.slice(0, 2)).toEqual(["Beta", "Alpha"]);
  });

  it("renders nothing while the first load is in flight", () => {
    mockUseChatThreads.mockReturnValue({ threads: [], isLoading: true, error: null });

    const { container } = render(<SidebarChatThreads pathname="/chats" />);

    expect(container).toBeEmptyDOMElement();
  });
});
