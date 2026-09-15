import { fireEvent, render, screen } from "@testing-library/react";
import WorkflowsPage from "@/app/(main)/durable/workflows/page";
import { DlqRow } from "@/app/(main)/durable/queues/dlq-row";
import { TaskRow } from "@/app/(main)/durable/queues/task-row";
import type { DlqEntry, DurableTask } from "@/lib/api/types";

const mockTask: DurableTask = {
  id: "task_123",
  workflow_id: "workflow_task",
  activity_id: "activity_123",
  activity_type: "send-email",
  status: "pending",
  priority: 1,
  scheduled_at: "2026-09-14T00:00:00Z",
  visible_at: "2026-09-14T00:00:00Z",
  attempt: 1,
  max_attempts: 3,
  created_at: "2026-09-14T00:00:00Z",
};

const mockDlqEntry: DlqEntry = {
  id: "dlq_123",
  original_task_id: "task_456",
  workflow_id: "workflow_dlq",
  activity_id: "activity_456",
  activity_type: "sync-index",
  input: {},
  attempts: 3,
  last_error: "request failed",
  error_history: ["request failed"],
  dead_at: "2026-09-14T00:00:00Z",
  requeue_count: 0,
};

jest.mock("@/hooks", () => ({
  usePageTitle: () => {},
  useWorkflows: () => ({
    data: { data: [], total: 0 },
    isLoading: false,
    error: null,
    refetch: jest.fn(),
  }),
  useCancelWorkflow: () => ({ mutate: jest.fn() }),
  useTasks: () => ({
    data: { data: [mockTask], total: 1 },
    isLoading: false,
    refetch: jest.fn(),
  }),
  useDlq: () => ({
    data: { data: [mockDlqEntry], total: 1 },
    isLoading: false,
    refetch: jest.fn(),
  }),
  useRequeueDlqEntry: () => ({ mutate: jest.fn() }),
}));

function expectWorkflowLink(name: string, workflowId: string) {
  const link = screen.getByRole("link", { name });
  expect(link).toHaveAttribute("href", `/durable/workflows/${workflowId}`);
  expect(link.querySelector("button")).not.toBeInTheDocument();
}

test("queue rows name icon-only workflow links by destination", () => {
  render(
    <table>
      <tbody>
        <TaskRow task={mockTask} />
        <DlqRow entry={mockDlqEntry} onRequeue={jest.fn()} onDelete={jest.fn()} />
      </tbody>
    </table>,
  );

  expectWorkflowLink("View workflow workflow_task", "workflow_task");
  expectWorkflowLink("View workflow workflow_dlq", "workflow_dlq");
});

test("workflow task and DLQ tabs name icon-only workflow links by destination", () => {
  render(<WorkflowsPage />);

  fireEvent.click(screen.getByRole("button", { name: "Tasks 1" }));
  expectWorkflowLink("View workflow workflow_task", "workflow_task");

  fireEvent.click(screen.getByRole("button", { name: "Dead Letter Queue 1" }));
  expectWorkflowLink("View workflow workflow_dlq", "workflow_dlq");
});
