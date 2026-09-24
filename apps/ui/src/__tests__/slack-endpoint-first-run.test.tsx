import { act, fireEvent, render, screen } from "@testing-library/react";
import { Suspense } from "react";
import NewAgentEndpointPage from "@/app/(main)/agents/[agentId]/endpoints/new/page";
import { AgentEndpointEditor } from "@/components/agents/agent-endpoint-editor";
import { ChannelForm, getDefaultChannelFormState } from "@/components/apps/channel-form";
import { beginSlackInstall } from "@/lib/api/agent-endpoints";
import type { Agent, AppChannel } from "@/lib/api/types";

const push = jest.fn();
const replace = jest.fn();
const createEndpoint = jest.fn();
let mockSlackInstallAvailable = true;

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
    data: { available: mockSlackInstallAvailable },
    isLoading: false,
  }),
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

describe("Slack endpoint first run", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSlackInstallAvailable = true;
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
        slackInstallAvailable={false}
      />,
    );
    expect(screen.getByLabelText("Signing secret")).toBeInTheDocument();
  });

  it("starts Slack install immediately after saving a credential-free endpoint", async () => {
    await renderNewEndpointPage();
    fireEvent.click(screen.getByRole("button", { name: /Slack/ }));

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save endpoint" }));
    });

    expect(beginSlackInstall).toHaveBeenCalledWith("appchan_123");
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

  it("keeps the manual save flow when no provisioner is available", async () => {
    mockSlackInstallAvailable = false;
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

    render(
      <AgentEndpointEditor
        agentId="agent_123"
        endpointId="appchan_123"
        slackInstallFailure="Slack rejected the app creation: ratelimited"
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Slack rejected the app creation: ratelimited",
    );
    expect(screen.getByRole("button", { name: "Connect to Slack" })).toBeInTheDocument();
  });

  it("does not suggest an unavailable reconnect action after install failure", () => {
    mockSlackInstallAvailable = false;
    render(
      <AgentEndpointEditor
        agentId="agent_123"
        endpointId="appchan_123"
        slackInstallFailure="Could not reach Slack to create the app"
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not reach Slack to create the app. Configure Slack manually.",
    );
    expect(screen.queryByText(/Use Connect to Slack/)).not.toBeInTheDocument();
  });
});
