import { fireEvent, render, screen } from "@testing-library/react";
import { ChatSwitcher } from "@/components/chat/chat-switcher";
import type { Session, SessionTask } from "@/lib/api/types";

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
  useChatThreads: () => ({
    threads: [
      { id: "main", title: "Chat", tags: ["chat"] },
      { id: "side", title: "Pricing wording", tags: ["chat"] },
    ] as Session[],
  }),
}));
jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTasks: () => ({
    data: [
      {
        id: "task_open",
        kind: "assignment",
        state: "running",
        display_name: "Research pricing",
        links: { child_session_id: "thread_open" },
        created_at: "2026-11-10T00:00:00Z",
        updated_at: "2026-11-10T00:00:00Z",
      },
      {
        id: "task_done",
        kind: "assignment",
        state: "succeeded",
        state_detail: "resolved",
        display_name: "Old work",
        links: { child_session_id: "thread_done" },
        created_at: "2026-11-09T00:00:00Z",
        updated_at: "2026-11-09T00:00:00Z",
      },
    ] as SessionTask[],
  }),
}));

it("switches chats and opens the open chat's unresolved threads", async () => {
  const onSelectChat = jest.fn();
  const onSelectThread = jest.fn();
  render(
    <ChatSwitcher
      mainChatId="main"
      activeChatId="main"
      onSelectChat={onSelectChat}
      onSelectThread={onSelectThread}
    />,
  );
  const trigger = screen.getByRole("button", { name: "Switch chat" });
  expect(trigger).toHaveTextContent("Main chat");
  fireEvent.click(trigger);
  await screen.findByRole("menu");
  expect(screen.queryByText("Old work")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("menuitem", { name: /Research pricing/ }));
  expect(onSelectThread).toHaveBeenCalledWith("thread_open");

  fireEvent.click(trigger);
  await screen.findByRole("menu");
  // Main chat is listed once, at the top, not again among side chats.
  expect(screen.getAllByRole("menuitem", { name: /Main chat|^Chat$/ })).toHaveLength(1);
  expect(screen.getByRole("menuitem", { name: "New side chat" })).toHaveAttribute(
    "href",
    "/chats/new",
  );
  fireEvent.click(screen.getByRole("menuitem", { name: "Pricing wording" }));
  expect(onSelectChat).toHaveBeenCalledWith("side");
});
