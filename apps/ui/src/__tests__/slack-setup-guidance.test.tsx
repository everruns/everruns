import { render, screen } from "@testing-library/react";
import { SlackSetupGuidance } from "@/components/apps/slack-setup-guidance";

describe("SlackSetupGuidance", () => {
  const baseProps = {
    hasSlackConfig: true,
    isPublished: false,
    webhookVerified: false,
    firstMessageReceived: false,
    webhookUrl: "https://example.com/api/v1/apps/app-123/slack/events",
    webhookPath: "/api/v1/apps/app-123/slack/events",
    isLocalhost: false,
    agentSurfaceEnabled: false,
    onCreateSlackApp: jest.fn(),
    creatingSlackApp: false,
    onConfigure: jest.fn(),
  };

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("keeps the setup checklist visible after publish", () => {
    render(<SlackSetupGuidance {...baseProps} isPublished={true} />);

    expect(screen.getByText("1. Publish the app")).toBeInTheDocument();
    expect(screen.getByText("2. Create a Slack App")).toBeInTheDocument();
    expect(screen.getByText("3. Copy credentials back")).toBeInTheDocument();
    expect(screen.getByText("4. Invite the bot and test")).toBeInTheDocument();
  });

  it("no longer asks the user to configure Event Subscriptions by hand", () => {
    render(<SlackSetupGuidance {...baseProps} isPublished={true} hasSlackConfig={false} />);

    expect(screen.queryByText(/Configure Event Subscriptions/)).not.toBeInTheDocument();
    // The manifest carries the subscriptions, so the URL is shown as information
    // rather than as something to paste into Slack.
    expect(screen.getByText(baseProps.webhookUrl)).toBeInTheDocument();
  });

  it("publishes before offering the manifest, and says why", () => {
    render(<SlackSetupGuidance {...baseProps} isPublished={false} hasSlackConfig={false} />);

    expect(screen.queryByRole("button", { name: "Create Slack App" })).not.toBeInTheDocument();
    expect(screen.getByText(/Available once the app is published/)).toBeInTheDocument();
  });

  it("shows create and configure actions once published", () => {
    render(<SlackSetupGuidance {...baseProps} isPublished={true} hasSlackConfig={false} />);

    expect(screen.getByRole("button", { name: "Create Slack App" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configure" })).toBeInTheDocument();
  });

  it("offers the agent surface as an upgrade that needs a reinstall, not a config flip", () => {
    render(<SlackSetupGuidance {...baseProps} isPublished={true} />);

    expect(screen.getByText("Agent surface available")).toBeInTheDocument();
    expect(screen.getByText(/reinstall it/)).toBeInTheDocument();
    expect(screen.getByText(/assistant:write/)).toBeInTheDocument();
  });

  it("still flags the reinstall once the surface is enabled", () => {
    render(<SlackSetupGuidance {...baseProps} isPublished={true} agentSurfaceEnabled={true} />);

    expect(screen.getByText("Agent surface enabled")).toBeInTheDocument();
    expect(screen.getByText(/still needs reinstalling/)).toBeInTheDocument();
  });

  it("does not mention the agent surface before Slack is configured at all", () => {
    render(<SlackSetupGuidance {...baseProps} hasSlackConfig={false} />);

    expect(screen.queryByText(/Agent surface/)).not.toBeInTheDocument();
  });

  it("keeps the manual Request URL path for localhost, which Slack cannot reach", () => {
    render(
      <SlackSetupGuidance
        {...baseProps}
        isPublished={true}
        hasSlackConfig={false}
        isLocalhost={true}
      />,
    );

    expect(screen.getByText(/ngrok http/)).toBeInTheDocument();
    expect(screen.getByText(baseProps.webhookPath)).toBeInTheDocument();
  });
});
