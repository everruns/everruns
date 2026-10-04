import { fireEvent, render, screen } from "@testing-library/react";
import { RunCard } from "@/components/chat/run-card";
import { ChatWorkspaceContext } from "@/components/chat/chat-workspace-context";
import type { SessionTask } from "@/lib/api/types";

const task = {
  id: "task_one",
  session_id: "session_main",
  display_name: "Work",
  kind: "external_agent",
  state: "running",
} as SessionTask;

it("opens permanent Chat work in the adopted panel", () => {
  const openTask = jest.fn();
  render(
    <ChatWorkspaceContext.Provider value={{ sessionId: "session_main", openTask }}>
      <RunCard run={{ task, subagentCount: 0 }} now={0} />
    </ChatWorkspaceContext.Provider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "View thread" }));
  expect(openTask).toHaveBeenCalledWith("task_one");
});

it("keeps recording navigation outside the adopted permanent conversation", () => {
  render(
    <ChatWorkspaceContext.Provider value={{ sessionId: "session_other", openTask: jest.fn() }}>
      <RunCard run={{ task, subagentCount: 0 }} now={0} />
    </ChatWorkspaceContext.Provider>,
  );
  expect(screen.getByRole("link", { name: "Open session" })).toHaveAttribute(
    "href",
    "/sessions/session_main/resources",
  );
  expect(screen.queryByRole("button", { name: "View thread" })).not.toBeInTheDocument();
});
