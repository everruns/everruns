import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AgentTriggersPanel } from "@/components/agents/agent-triggers-panel";
import type { AgentTrigger } from "@/lib/api/types";

const update = jest.fn();
const run = jest.fn();
const remove = jest.fn();
let canManage = true;
let hideSchedule = false;
jest.mock("@/components/ui/confirmation-provider", () => ({
  useConfirmation: () => async () => true,
}));
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href, ...rest }: { children: React.ReactNode; href: string }) => (
    <a href={href} {...rest}>
      {children}
    </a>
  ),
}));
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => canManage, isLoading: false, error: null }),
}));

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
      {
        id: "trg_github",
        agent_id: "agent_123",
        trigger_type: "github",
        config: {
          events: ["pull_request.opened"],
          repositories: ["acme/api"],
          session_mode: "per_thread",
          message: "Review the PR",
        },
        enabled: true,
        created_at: "2026-07-17T00:00:00Z",
        updated_at: "2026-07-17T00:00:00Z",
      } as unknown as AgentTrigger,
      {
        id: "trg_mcp",
        agent_id: "agent_123",
        trigger_type: "mcp_event",
        ingress_id: "appchan_mcp",
        config: {
          server: "tracker",
          event: "issue.created",
          arguments: { team: "eng" },
          session_mode: "shared_session",
          message: "Triage the issue",
        },
        enabled: true,
        created_at: "2026-07-17T00:00:00Z",
        updated_at: "2026-07-17T00:00:00Z",
      } as unknown as AgentTrigger,
    ].filter((trigger) => !hideSchedule || trigger.id !== "trg_123"),
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
  useUpdateAgentTrigger: () => ({ mutate: update, isPending: false }),
  useDeleteAgentTrigger: () => ({ mutate: remove, isPending: false }),
  useRunAgentTrigger: () => ({ mutate: run, isPending: false }),
}));

describe("AgentTriggersPanel", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    canManage = true;
    hideSchedule = false;
  });

  it("lists every trigger as a row that opens its page", () => {
    render(<AgentTriggersPanel agentId="agent_123" />);

    const table = screen.getByRole("table", { name: "Triggers" });
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((h) => h.textContent),
    ).toEqual(["Trigger", "Status", "Last run"]);
    expect(screen.getByRole("link", { name: "At 30 minutes past the hour" })).toHaveAttribute(
      "href",
      "/agents/agent_123/triggers/trg_123",
    );
    expect(screen.getByRole("link", { name: "Webhook" })).toHaveAttribute(
      "href",
      "/agents/agent_123/triggers/trg_webhook",
    );
    expect(screen.getByRole("link", { name: "Pull request events" })).toHaveAttribute(
      "href",
      "/agents/agent_123/triggers/trg_github",
    );
    expect(screen.getByRole("link", { name: "issue.created on tracker" })).toHaveAttribute(
      "href",
      "/agents/agent_123/triggers/trg_mcp",
    );
    expect(
      screen.getByText(/Schedule · America\/Chicago · Prepare the hourly report/),
    ).toBeInTheDocument();
    expect(screen.getByText(/GitHub · acme\/api · pull_request\.opened/)).toBeInTheDocument();
    // Setup (URLs, tokens) lives on the trigger's page, not in the list.
    expect(screen.queryByText(/appchan_webhook/)).not.toBeInTheDocument();
  });

  it("runs, turns off and deletes from the row menu", async () => {
    render(<AgentTriggersPanel agentId="agent_123" />);

    const menu = () =>
      screen.getByRole("button", { name: "Actions for At 30 minutes past the hour" });
    fireEvent.click(menu());
    fireEvent.click(await screen.findByRole("menuitem", { name: "Run now" }));
    expect(run).toHaveBeenCalledWith("trg_123");

    fireEvent.click(menu());
    fireEvent.click(await screen.findByRole("menuitem", { name: "Turn off" }));
    expect(update).toHaveBeenCalledWith({ triggerId: "trg_123", request: { enabled: false } });

    fireEvent.click(menu());
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("trg_123"));
  });

  it("offers no actions or Add trigger without agent management permission", () => {
    canManage = false;
    render(<AgentTriggersPanel agentId="agent_123" />);

    expect(screen.queryByRole("button", { name: /^Actions for/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Add trigger" })).not.toBeInTheDocument();
  });

  it("links to the full-page form to create a trigger", () => {
    render(<AgentTriggersPanel agentId="agent_123" />);

    expect(screen.getByRole("link", { name: "Add trigger" })).toHaveAttribute(
      "href",
      "/agents/agent_123/triggers/new",
    );
  });
});
