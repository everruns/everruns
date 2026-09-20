import { render, screen } from "@testing-library/react";
import { WebhookSetupGuidance } from "@/components/agents/integrations/webhook-setup-guidance";

describe("WebhookSetupGuidance", () => {
  it("renders the endpoint and auth guidance", () => {
    render(
      <WebhookSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/webhooks/appchan-123"
        sessionMode="session_per_invocation"
        message="Process {{payload.repo.name}}"
        tokenConfigured={true}
        isEnabled={true}
      />,
    );

    expect(screen.getByText("Token Configured")).toBeInTheDocument();
    expect(
      screen.getByText("https://example.com/api/v1/apps/app-123/webhooks/appchan-123"),
    ).toBeInTheDocument();
    expect(screen.getByText("Session Per Invocation")).toBeInTheDocument();
    // Auth guidance plus the generated code samples and coding-agent prompt all
    // reference the bearer header, so there are multiple matches by design.
    expect(screen.getAllByText(/Authorization: Bearer/).length).toBeGreaterThan(0);
    // Integration snippets are surfaced for callers.
    expect(screen.getByText("Call it from your system")).toBeInTheDocument();
    expect(screen.getByText("Let a coding agent wire it up")).toBeInTheDocument();
  });
});
