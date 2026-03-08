import { render, screen } from "@testing-library/react";
import { SlackSetupGuidance } from "@/components/apps/slack-setup-guidance";

describe("SlackSetupGuidance", () => {
  const baseProps = {
    hasSlackConfig: true,
    isPublished: false,
    webhookUrl: "https://example.com/api/v1/apps/app-123/slack/events",
    webhookPath: "/api/v1/apps/app-123/slack/events",
    isLocalhost: false,
    onCreateSlackApp: jest.fn(),
    creatingSlackApp: false,
    onConfigure: jest.fn(),
  };

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("keeps the setup checklist visible after publish", () => {
    render(<SlackSetupGuidance {...baseProps} isPublished={true} />);

    expect(screen.getByText("3. Publish the app")).toBeInTheDocument();
    expect(screen.getByText("4. Configure Event Subscriptions")).toBeInTheDocument();
    expect(screen.getByText(baseProps.webhookUrl)).toBeInTheDocument();
    expect(screen.getByText("5. Invite the bot and test")).toBeInTheDocument();
  });

  it("shows create and configure actions before Slack credentials exist", () => {
    render(<SlackSetupGuidance {...baseProps} hasSlackConfig={false} />);

    expect(screen.getByRole("button", { name: "Create Slack App" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configure" })).toBeInTheDocument();
  });
});
