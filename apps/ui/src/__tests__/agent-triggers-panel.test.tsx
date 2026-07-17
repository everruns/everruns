import { fireEvent, render, screen } from "@testing-library/react";
import { AgentTriggersPanel } from "@/components/agents/agent-triggers-panel";
import type { AgentTrigger } from "@/lib/api/types";

const update = jest.fn();
const run = jest.fn();
const remove = jest.fn();

jest.mock("@/hooks/use-agent-triggers", () => ({
  useAgentTriggers: () => ({
    data: [
      {
        id: "trg_123",
        agent_id: "agent_123",
        trigger_type: "schedule",
        config: {
          cron_expression: "0 30 * * * * *",
          timezone: "America/Chicago",
          session_mode: "session_per_invocation",
          message: "Prepare the hourly report",
        },
        enabled: true,
        created_at: "2026-07-17T00:00:00Z",
        updated_at: "2026-07-17T00:00:00Z",
      } satisfies AgentTrigger,
    ],
    isLoading: false,
  }),
  useAgentTriggerRuns: () => ({
    data: [
      {
        id: "exec_123",
        status: "completed",
        scheduled_at: new Date().toISOString(),
        completed_at: new Date().toISOString(),
      },
    ],
    isLoading: false,
  }),
  useCreateAgentTrigger: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useUpdateAgentTrigger: () => ({ mutate: update, mutateAsync: jest.fn(), isPending: false }),
  useDeleteAgentTrigger: () => ({ mutate: remove }),
  useRunAgentTrigger: () => ({ mutate: run, isPending: false }),
}));

describe("AgentTriggersPanel", () => {
  beforeEach(() => jest.clearAllMocks());

  it("shows schedule details, recent outcomes, and trigger actions", () => {
    render(<AgentTriggersPanel agentId="agent_123" />);

    expect(screen.getByText("At 30 minutes past the hour · America/Chicago")).toBeInTheDocument();
    expect(screen.getByText("Prepare the hourly report")).toBeInTheDocument();
    expect(screen.getByText("New session per run")).toBeInTheDocument();
    expect(screen.getByText("completed")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Run now" }));
    fireEvent.click(screen.getByRole("switch", { name: "Disable trigger" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete trigger" }));

    expect(run).toHaveBeenCalledWith("trg_123");
    expect(update).toHaveBeenCalledWith({ triggerId: "trg_123", request: { enabled: false } });
    expect(remove).toHaveBeenCalledWith("trg_123");
  });

  it("opens the create form", () => {
    render(<AgentTriggersPanel agentId="agent_123" />);

    fireEvent.click(screen.getByRole("button", { name: "Add trigger" }));

    expect(screen.getByRole("dialog", { name: "Add trigger" })).toBeInTheDocument();
    expect(screen.getByLabelText("Cron expression")).toBeInTheDocument();
    expect(screen.getByLabelText("Message")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Add trigger" })).not.toBeInTheDocument();
  });
});
