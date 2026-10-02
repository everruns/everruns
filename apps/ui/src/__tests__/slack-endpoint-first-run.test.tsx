import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Suspense } from "react";
import EditAgentEndpointPage from "@/app/(main)/agents/[agentId]/endpoints/[endpointId]/page";
import NewAgentEndpointPage from "@/app/(main)/agents/[agentId]/endpoints/new/page";
import { ChannelForm, getDefaultChannelFormState } from "@/components/apps/channel-form";
import { beginSlackInstall } from "@/lib/api/agent-endpoints";
import type { SlackInstallCapability, SlackWorkspace } from "@/lib/api/agent-endpoints";
import { classifySlackConfigToken } from "@/components/slack/slack-workspaces";
import type { Agent, AppChannel } from "@/lib/api/types";

const push = jest.fn();
const replace = jest.fn();
const createEndpoint = jest.fn();
let mockSlackCapability: SlackInstallCapability = {
  supported: true,
  connected: true,
  reconnect_required: false,
  can_manage: true,
};

function workspace(teamId: string, name: string, status: SlackWorkspace["status"] = "connected") {
  return {
    id: `ws_${teamId}`,
    team_id: teamId,
    team_name: name,
    status,
    connected_at: "2026-09-30T00:00:00Z",
  } satisfies SlackWorkspace;
}

let mockWorkspaces: SlackWorkspace[] = [workspace("T1", "Acme")];

jest.mock("next/navigation", () => ({
  usePathname: () => "/agents/agent_123/endpoints/new",
  useRouter: () => ({ push, replace }),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href, ...props }: React.ComponentPropsWithoutRef<"a">) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => false,
}));

jest.mock("@/hooks/use-agents", () => ({
  useAgent: () => ({
    data: {
      id: "agent_123",
      name: "support-agent",
      display_name: "Support Agent",
      status: "active",
    } as Agent,
    isLoading: false,
    refetch: jest.fn(),
  }),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({
    can: () => true,
    isLoading: false,
  }),
}));

jest.mock("@/hooks/use-agent-endpoints", () => ({
  useAgentEndpoints: () => ({
    endpoints: [{ channel: slackEndpoint() }],
    isLoading: false,
  }),
  useCreateAgentEndpoint: () => ({
    mutate: createEndpoint,
    isPending: false,
  }),
  useSlackInstallCapability: () => ({
    data: mockSlackCapability,
    isLoading: false,
    refetch: jest.fn(),
  }),
  useSlackWorkspaces: () => ({ data: mockWorkspaces, isLoading: false }),
  useInvalidateSlackWorkspaces: () => jest.fn(),
  useUpdateAgentEndpoint: () => ({ mutate: jest.fn(), isPending: false }),
  useDeleteAgentEndpoint: () => ({ mutate: jest.fn(), isPending: false }),
  usePublishAgentEndpoint: () => ({ mutate: jest.fn(), isPending: false }),
  useTriggerAgentEndpoint: () => ({ mutate: jest.fn(), isPending: false }),
}));

jest.mock("@/lib/api/agent-endpoints", () => ({
  ...jest.requireActual("@/lib/api/agent-endpoints"),
  beginSlackInstall: jest.fn(),
}));

function slackEndpoint(): AppChannel {
  return {
    id: "appchan_123",
    channel_type: "slack",
    channel_config: {},
    enabled: true,
    status: "draft",
    created_at: "2026-09-23T00:00:00Z",
    updated_at: "2026-09-23T00:00:00Z",
  };
}

async function renderNewEndpointPage() {
  const params = Promise.resolve({ agentId: "agent_123" });
  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <NewAgentEndpointPage params={params} />
      </Suspense>,
    );
    await params;
  });
}
async function renderEditEndpointPage(reason: string) {
  const params = Promise.resolve({
    agentId: "agent_123",
    endpointId: "appchan_123",
  });
  const searchParams = Promise.resolve({ slack_install: "failed", reason });
  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <EditAgentEndpointPage params={params} searchParams={searchParams} />
      </Suspense>,
    );
    await Promise.all([params, searchParams]);
  });
}
describe("Slack endpoint first run", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    window.history.replaceState(null, "", "/agents/agent_123/endpoints/new");
    mockSlackCapability = {
      supported: true,
      connected: true,
      reconnect_required: false,
      can_manage: true,
    };
    mockWorkspaces = [workspace("T1", "Acme")];
    (beginSlackInstall as jest.Mock).mockImplementation(() => new Promise(() => undefined));
    createEndpoint.mockImplementation(
      (_request: unknown, options: { onSuccess: (endpoint: AppChannel) => void }) =>
        options.onSuccess(slackEndpoint()),
    );
  });

  it("starts with manual Slack credentials collapsed", () => {
    render(
      <ChannelForm state={getDefaultChannelFormState("slack")} onChange={jest.fn()} mode="new" />,
    );

    expect(screen.queryByLabelText("Signing secret")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Bot token")).not.toBeInTheDocument();
  });

  it("opens manual fields for saved credentials and deployments without a provisioner", () => {
    const { unmount } = render(
      <ChannelForm
        state={getDefaultChannelFormState("slack", {
          ...slackEndpoint(),
          channel_config: { bot_token_configured: true },
        })}
        onChange={jest.fn()}
        mode="edit"
        endpointId="appchan_123"
      />,
    );

    expect(screen.getByLabelText("Bot token")).toBeInTheDocument();
    unmount();

    render(
      <ChannelForm
        state={getDefaultChannelFormState("slack")}
        onChange={jest.fn()}
        mode="new"
        slackInstallCapability={{
          supported: false,
          connected: false,
          reconnect_required: false,
          can_manage: false,
        }}
      />,
    );
    expect(screen.getByLabelText("Signing secret")).toBeInTheDocument();
  });

  it("guides an administrator through connecting a workspace when none is connected", () => {
    mockWorkspaces = [];
    const { unmount } = render(
      <ChannelForm
        state={getDefaultChannelFormState("slack")}
        onChange={jest.fn()}
        mode="new"
        slackInstallCapability={{ ...mockSlackCapability, connected: false }}
      />,
    );
    expect(screen.getByText("Connect a Slack workspace")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open api\.slack\.com\/apps/ })).toHaveAttribute(
      "href",
      "https://api.slack.com/apps",
    );
    expect(screen.getByLabelText(/paste it here/)).toHaveAttribute("type", "password");
    expect(screen.queryByLabelText("Signing secret")).not.toBeInTheDocument();
    expect(screen.getByText(/can create and change any Slack app in that workspace/)).toBeVisible();
    unmount();

    render(
      <ChannelForm
        state={getDefaultChannelFormState("slack")}
        onChange={jest.fn()}
        mode="new"
        slackInstallCapability={{
          ...mockSlackCapability,
          connected: false,
          can_manage: false,
        }}
      />,
    );
    expect(screen.getByText(/Ask an organization administrator/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Settings → Slack workspaces" })).toHaveAttribute(
      "href",
      "/settings/slack",
    );
    expect(screen.queryByLabelText(/paste it here/)).not.toBeInTheDocument();
  });

  it("distinguishes reconnect-required from first-time setup", () => {
    mockWorkspaces = [workspace("T1", "Acme", "reconnect_required")];
    render(
      <ChannelForm
        state={getDefaultChannelFormState("slack")}
        onChange={jest.fn()}
        mode="new"
        slackInstallCapability={{
          ...mockSlackCapability,
          connected: false,
          reconnect_required: true,
        }}
      />,
    );
    expect(screen.getByText("Reconnect your Slack workspace")).toBeInTheDocument();
  });

  it("names the access-token mistake instead of submitting it", () => {
    expect(classifySlackConfigToken("")).toBe("empty");
    expect(classifySlackConfigToken("  xoxe-1-abc ")).toBe("refresh");
    expect(classifySlackConfigToken("xoxe.xoxp-1-abc")).toBe("access");
    expect(classifySlackConfigToken("xoxb-123")).toBe("unknown");

    mockWorkspaces = [];
    render(
      <ChannelForm
        state={getDefaultChannelFormState("slack")}
        onChange={jest.fn()}
        mode="new"
        slackInstallCapability={{ ...mockSlackCapability, connected: false }}
      />,
    );
    fireEvent.change(screen.getByLabelText(/paste it here/), {
      target: { value: "xoxe.xoxp-1-access" },
    });
    expect(screen.getByText(/That is the access token/)).toBeInTheDocument();
  });

  it("shows the only workspace and selects it without asking", () => {
    const onChange = jest.fn();
    render(
      <ChannelForm
        state={getDefaultChannelFormState("slack")}
        onChange={onChange}
        mode="new"
        slackInstallCapability={mockSlackCapability}
      />,
    );
    expect(screen.getByText("Acme (T1)")).toBeInTheDocument();
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ slackInstallTeamId: "T1" }));
  });

  it("asks which workspace when several are connected", () => {
    mockWorkspaces = [workspace("T1", "Acme"), workspace("T2", "Globex")];
    const onChange = jest.fn();
    render(
      <ChannelForm
        state={getDefaultChannelFormState("slack")}
        onChange={onChange}
        mode="edit"
        endpointId="appchan_123"
        slackInstallCapability={mockSlackCapability}
      />,
    );
    expect(screen.getByLabelText("Slack workspace")).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Add to Slack" })).toBeDisabled();
  });

  it("links into Slack once the agent's app is installed", () => {
    render(
      <ChannelForm
        state={getDefaultChannelFormState("slack", {
          ...slackEndpoint(),
          channel_config: {
            slack_app_id: "A0123",
            team_id: "T1",
            bot_token_configured: true,
            session_strategy: "per_thread",
          },
        })}
        onChange={jest.fn()}
        mode="edit"
        endpointId="appchan_123"
        slackInstallCapability={mockSlackCapability}
      />,
    );
    expect(screen.getByText("Live in Slack")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open in Slack/ })).toHaveAttribute(
      "href",
      "https://slack.com/app_redirect?app=A0123&team=T1",
    );
    expect(screen.queryByRole("button", { name: "Add to Slack" })).not.toBeInTheDocument();
  });

  it("starts Slack install immediately after saving a credential-free endpoint", async () => {
    const authorizeUrl = `${window.location.href}#slack-consent`;
    (beginSlackInstall as jest.Mock).mockResolvedValue({
      authorize_url: authorizeUrl,
    });
    await renderNewEndpointPage();
    fireEvent.click(screen.getByRole("button", { name: /Slack/ }));

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save endpoint" }));
    });
    await waitFor(() => expect(window.location.href).toBe(authorizeUrl));
    expect(beginSlackInstall).toHaveBeenCalledWith("appchan_123", "T1");
  });

  it("does not hijack manually entered credentials", async () => {
    await renderNewEndpointPage();
    fireEvent.click(screen.getByRole("button", { name: /Slack/ }));
    fireEvent.click(screen.getByRole("button", { name: "Configure manually" }));
    fireEvent.change(screen.getByLabelText("Signing secret"), {
      target: { value: "manual-secret" },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save endpoint" }));
    });

    expect(beginSlackInstall).not.toHaveBeenCalled();
    expect(push).toHaveBeenCalledWith("/agents/agent_123/endpoints/appchan_123");
  });

  it("asks for a workspace before saving when several are connected", async () => {
    mockWorkspaces = [workspace("T1", "Acme"), workspace("T2", "Globex")];
    await renderNewEndpointPage();
    fireEvent.click(screen.getByRole("button", { name: /Slack/ }));

    expect(screen.getByRole("button", { name: "Save endpoint" })).toBeDisabled();
    expect(beginSlackInstall).not.toHaveBeenCalled();
  });

  it("keeps the manual save flow when no provisioner is available", async () => {
    mockSlackCapability = {
      supported: false,
      connected: false,
      reconnect_required: false,
      can_manage: false,
    };
    await renderNewEndpointPage();
    fireEvent.click(screen.getByRole("button", { name: /Slack/ }));
    expect(screen.getByLabelText("Signing secret")).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save endpoint" }));
    });

    expect(beginSlackInstall).not.toHaveBeenCalled();
    expect(push).toHaveBeenCalledWith("/agents/agent_123/endpoints/appchan_123");
  });

  it("lands on the saved endpoint with a visible install failure reason", async () => {
    (beginSlackInstall as jest.Mock).mockRejectedValue(
      new Error("Slack rejected the app creation: ratelimited"),
    );
    await renderNewEndpointPage();
    fireEvent.click(screen.getByRole("button", { name: /Slack/ }));

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save endpoint" }));
    });

    expect(push).toHaveBeenCalledWith(
      "/agents/agent_123/endpoints/appchan_123?slack_install=failed&reason=Slack%20rejected%20the%20app%20creation%3A%20ratelimited",
    );

    await renderEditEndpointPage("Slack rejected the app creation: ratelimited");

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Slack rejected the app creation: ratelimited",
    );
    expect(screen.getByRole("button", { name: "Add to Slack" })).toBeInTheDocument();
  });

  it("does not suggest an unavailable reconnect action after install failure", async () => {
    mockSlackCapability = {
      supported: false,
      connected: false,
      reconnect_required: false,
      can_manage: false,
    };
    await renderEditEndpointPage("Could not reach Slack to create the app");

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not reach Slack to create the app. Configure Slack manually.",
    );
    expect(screen.queryByText(/Use Add to Slack/)).not.toBeInTheDocument();
  });
});
