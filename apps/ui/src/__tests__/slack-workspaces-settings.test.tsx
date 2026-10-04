import { fireEvent, render, screen } from "@testing-library/react";
import SlackWorkspacesPage from "@/app/(main)/settings/slack/page";
import type { SlackInstallCapability, SlackWorkspace } from "@/lib/api/agent-channels";

let mockCapability: SlackInstallCapability;
let mockWorkspaces: SlackWorkspace[];

jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));

jest.mock("@/hooks/use-agent-channels", () => ({
  useSlackInstallCapability: () => ({ data: mockCapability, isLoading: false }),
  useSlackWorkspaces: () => ({ data: mockWorkspaces, isLoading: false }),
  useInvalidateSlackWorkspaces: () => jest.fn(),
}));

function workspace(teamId: string, name: string | null, status: SlackWorkspace["status"]) {
  return {
    id: `ws_${teamId}`,
    team_id: teamId,
    team_name: name,
    status,
    connected_at: "2026-09-30T00:00:00Z",
  } satisfies SlackWorkspace;
}

describe("Slack workspaces settings", () => {
  beforeEach(() => {
    mockCapability = {
      supported: true,
      connected: true,
      reconnect_required: false,
      can_manage: true,
    };
    mockWorkspaces = [
      workspace("T1", "Acme", "connected"),
      workspace("T2", null, "reconnect_required"),
    ];
  });

  it("lists every connected workspace with its state", () => {
    render(<SlackWorkspacesPage />);
    expect(screen.getByText("Acme (T1)")).toBeInTheDocument();
    // A workspace whose name Slack would not reveal falls back to its id.
    expect(screen.getByText("T2")).toBeInTheDocument();
    expect(screen.getByText("Reconnect required")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Connect another workspace/ })).toBeInTheDocument();
  });

  it("says running agents survive before disconnecting", () => {
    render(<SlackWorkspacesPage />);
    fireEvent.click(screen.getAllByRole("button", { name: "Disconnect" })[0]);
    expect(screen.getByText(/keep working — each has its own Slack app/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Disconnect workspace" })).toBeInTheDocument();
  });

  it("shows members the list without management actions", () => {
    mockCapability = { ...mockCapability, can_manage: false };
    render(<SlackWorkspacesPage />);
    expect(screen.getByText("Acme (T1)")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Disconnect" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Connect another/ })).not.toBeInTheDocument();
  });

  it("explains a deployment that does not create Slack apps", () => {
    mockCapability = {
      supported: false,
      connected: false,
      reconnect_required: false,
      can_manage: false,
    };
    mockWorkspaces = [];
    render(<SlackWorkspacesPage />);
    expect(screen.getByText(/does not create Slack apps for you/)).toBeInTheDocument();
  });
});
