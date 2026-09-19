import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { EndpointDetailsPanel } from "@/components/agents/integrations/endpoint-details-panel";
import { getSlackEndpointManifest } from "@/lib/api/agent-endpoints";
import type { AppChannel } from "@/lib/api/types";

const mockPush = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush }),
}));

jest.mock("@/lib/api/agent-endpoints", () => ({
  getSlackEndpointManifest: jest.fn(),
}));

function channel(overrides: Partial<AppChannel>): AppChannel {
  return {
    id: "appchan_123",
    channel_type: "slack",
    channel_config: {
      session_strategy: "per_thread",
    },
    enabled: true,
    status: "live",
    created_at: "2026-09-19T00:00:00Z",
    updated_at: "2026-09-19T00:00:00Z",
    ...overrides,
  };
}

describe("EndpointDetailsPanel", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(window, "open").mockImplementation(() => null);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("shows setup before use and opens the endpoint-scoped Slack manifest", async () => {
    (getSlackEndpointManifest as jest.Mock).mockResolvedValue({
      manifest_yaml: `settings:
  event_subscriptions:
    request_url: "https://everruns.example/api/v1/e/appchan_123/slack/events"`,
      create_url: "https://api.slack.com/apps?new_app=1&manifest_yaml=encoded",
    });

    render(
      <EndpointDetailsPanel
        agentName="Support Agent"
        channel={channel({})}
        configureHref="/agents/agent_123/endpoints/appchan_123"
      />,
    );

    const setup = screen.getByText("Set up");
    const use = screen.getByText("Use it");
    expect(setup.compareDocumentPosition(use) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);

    await waitFor(() => expect(getSlackEndpointManifest).toHaveBeenCalledWith("appchan_123"));
    const createButton = screen.getByRole("button", { name: "Create Slack app" });
    await waitFor(() => expect(createButton).toBeEnabled());
    fireEvent.click(createButton);
    expect(window.open).toHaveBeenCalledWith(
      "https://api.slack.com/apps?new_app=1&manifest_yaml=encoded",
      "_blank",
      "noopener,noreferrer",
    );
  });
  it("keeps Slack app creation disabled when the manifest uses localhost", async () => {
    (getSlackEndpointManifest as jest.Mock).mockResolvedValue({
      manifest_yaml: `settings:
  event_subscriptions:
    request_url: "http://localhost:9300/api/v1/e/appchan_123/slack/events"`,
      create_url: "https://api.slack.com/apps?new_app=1&manifest_yaml=encoded",
    });

    render(<EndpointDetailsPanel agentName="Support Agent" channel={channel({})} />);

    const createButton = screen.getByRole("button", { name: "Create Slack app" });
    await waitFor(() => expect(screen.getByText(/PUBLIC_APP_URL/)).toBeInTheDocument());
    expect(createButton).toBeDisabled();
    fireEvent.click(createButton);
    expect(window.open).not.toHaveBeenCalled();
  });

  it("mounts the A2A Agent Card setup path in the expanded endpoint details", () => {
    render(
      <EndpointDetailsPanel
        agentName="Support Agent"
        agentDescription="Answers support questions"
        channel={channel({
          channel_type: "a2a",
          status: "draft",
          channel_config: {
            api_key_prefix: "evra2a_12345678...",
            session_mode: "shared_session",
            message: "Handle {{a2a.text}}",
          },
        })}
      />,
    );

    expect(screen.getByText("Agent Card")).toBeInTheDocument();
    expect(screen.getByText(/Publish and enable this endpoint/)).toBeInTheDocument();
    expect(screen.getByText("Use it")).toBeInTheDocument();
  });

  it.each([
    [
      "AG-UI",
      channel({
        channel_type: "ag_ui",
        channel_config: { anonymous: true, session_expiration_seconds: 3600 },
      }),
      "Image upload",
    ],
    [
      "FCP",
      channel({
        channel_type: "fcp",
        channel_config: { anonymous: true, session_expiration_seconds: 3600 },
      }),
      "Handshake (GET body)",
    ],
  ])("mounts %s setup guidance", (_name, endpoint, expectedText) => {
    render(<EndpointDetailsPanel agentName="Support Agent" channel={endpoint} />);

    expect(screen.getByText(expectedText)).toBeInTheDocument();
    expect(screen.getByText("Use it")).toBeInTheDocument();
  });
});
