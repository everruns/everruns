import { render, screen } from "@testing-library/react";
import { WebhookSetupGuidance } from "@/components/apps/webhook-setup-guidance";

describe("WebhookSetupGuidance", () => {
  it("renders the endpoint and auth guidance", () => {
    render(
      <WebhookSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/webhooks/appchan-123"
        sessionMode="session_per_invocation"
        message="Process {{payload.repo.name}}"
        tokenConfigured={true}
        isPublished={true}
      />,
    );

    expect(screen.getByText("Token Configured")).toBeInTheDocument();
    expect(screen.getByText("https://example.com/api/v1/apps/app-123/webhooks/appchan-123")).toBeInTheDocument();
    expect(screen.getByText("Session Per Invocation")).toBeInTheDocument();
    expect(screen.getByText(/Authorization: Bearer/)).toBeInTheDocument();
  });
});
