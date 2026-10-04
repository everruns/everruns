import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ChatThreadWorkDetail } from "@/components/chat/chat-thread-work-detail";
import type { SessionTask } from "@/lib/api/types";

const mockSend = jest.fn();
const mockCancel = jest.fn();
jest.mock("@/hooks/use-session-tasks", () => ({
  useSendTaskMessage: () => ({ mutateAsync: mockSend, isPending: false }),
  useCancelSessionTask: () => ({ mutate: mockCancel, isPending: false }),
}));
jest.mock("@/components/session/task-detail", () => ({ TaskCard: () => <div>Work</div> }));

const task = {
  id: "task_one",
  session_id: "session_one",
  kind: "external_agent",
  state: "awaiting_input",
  input_request: { id: "request_one", prompt: "Choose a name" },
} as SessionTask;

beforeEach(() => {
  mockSend.mockReset();
  mockCancel.mockReset();
});

it("answers the exact pending request and clears the draft only after success", async () => {
  mockSend.mockResolvedValue({});
  render(<ChatThreadWorkDetail task={task} />);
  fireEvent.change(screen.getByLabelText("Answer the request"), {
    target: { value: "  chosen  " },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() =>
    expect(mockSend).toHaveBeenCalledWith({
      taskId: "task_one",
      request: {
        content: [{ type: "text", text: "chosen" }],
        in_reply_to: "request_one",
      },
    }),
  );
  await waitFor(() => expect(screen.getByLabelText("Answer the request")).toHaveValue(""));
});

it("preserves a failed answer for retry", async () => {
  mockSend.mockRejectedValue(new Error("Offline"));
  render(<ChatThreadWorkDetail task={task} />);
  fireEvent.change(screen.getByLabelText("Answer the request"), { target: { value: "chosen" } });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(mockSend).toHaveBeenCalled());
  expect(screen.getByLabelText("Answer the request")).toHaveValue("chosen");
});

it("allows cancellation but never offers unsupported subagent messages", () => {
  render(<ChatThreadWorkDetail task={{ ...task, kind: "subagent" }} />);
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Stop work" }));
  expect(mockCancel).toHaveBeenCalledWith(task.id);
});

it("does not mutate completed work", () => {
  render(<ChatThreadWorkDetail task={{ ...task, state: "succeeded" }} />);
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Stop work" })).not.toBeInTheDocument();
});
