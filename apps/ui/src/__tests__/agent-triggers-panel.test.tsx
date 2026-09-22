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
      {
        id: "trg_webhook",
        agent_id: "agent_123",
        trigger_type: "webhook",
        ingress_id: "appchan_webhook",
        config: {
          token_configured: true,
          session_mode: "shared_session",
          message: "Process {{payload}}",
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
    expect(screen.getAllByText("Prepare the hourly report").length).toBeGreaterThan(0);
    expect(screen.getByText("New session per run")).toBeInTheDocument();
    expect(screen.getByText("completed")).toBeInTheDocument();
    expect(screen.getByText("Webhook")).toBeInTheDocument();
    expect(screen.getByText("Token Configured")).toBeInTheDocument();
    expect(screen.getAllByText(/\/api\/v1\/e\/appchan_webhook\/webhook/).length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "Run now" }));
    fireEvent.click(screen.getAllByRole("switch", { name: "Disable trigger" })[0]);
    fireEvent.click(screen.getAllByRole("button", { name: "Delete trigger" })[0]);

    expect(run).toHaveBeenCalledWith("trg_123");
    expect(update).toHaveBeenCalledWith({ triggerId: "trg_123", request: { enabled: false } });
    expect(remove).toHaveBeenCalledWith("trg_123");
  });

  // Creating a trigger is a full-page route since EVE-1009, so the panel links
  // rather than opening a dialog. The dialog is quick edit only.
  it("links to the full-page editor to create a trigger", () => {
    render(<AgentTriggersPanel agentId="agent_123" />);

    expect(screen.getByRole("link", { name: "Add trigger" })).toHaveAttribute(
      "href",
      "/agents/agent_123/triggers/new",
    );
    expect(screen.queryByRole("button", { name: "Add trigger" })).not.toBeInTheDocument();
  });
});
