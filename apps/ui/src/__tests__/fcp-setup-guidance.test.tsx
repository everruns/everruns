import { render, screen } from "@testing-library/react";
import { FcpSetupGuidance, formatFcpSessionExpiration } from "@/components/apps/fcp-setup-guidance";

describe("FcpSetupGuidance", () => {
  it("renders endpoint, anonymous badge, and timeout for a published app", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={6 * 60 * 60}
        responseTimeoutSeconds={120}
      />,
    );

    expect(screen.getByText("Anonymous")).toBeInTheDocument();
    expect(screen.getByText("Ready for FCP clients")).toBeInTheDocument();
    expect(screen.getByText("https://example.com/api/v1/apps/app-123/fcp")).toBeInTheDocument();
    expect(screen.getByText(/Auto-generated/)).toBeInTheDocument();
    expect(screen.getByText(/120 seconds/)).toBeInTheDocument();
  });

  it("renders 'Never' when session expiration is disabled", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={0}
      />,
    );

    expect(screen.getByText(/Never.*sessions can be resumed indefinitely/)).toBeInTheDocument();
  });

  it("renders 'Token Protected' badge and token value when token is provided", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={6 * 60 * 60}
        token="fcp-secret-12345"
      />,
    );

    expect(screen.getByText("fcp-secret-12345")).toBeInTheDocument();
    expect(screen.getByText("Token Protected")).toBeInTheDocument();
    // The token requirement guidance is rendered when a token is present.
    expect(screen.getByText(/Authorization: Bearer/i)).toBeInTheDocument();
  });

  it("renders 'Token Protected' when token is configured but not surfaced", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={false}
        anonymousEnabled={true}
        sessionExpirationSeconds={6 * 60 * 60}
        tokenConfigured={true}
      />,
    );

    expect(screen.getByText("Token Protected")).toBeInTheDocument();
    expect(screen.getByText("Channel token configured")).toBeInTheDocument();
    expect(screen.getByText("Publish the app to accept requests")).toBeInTheDocument();
  });

  it("renders 'Restricted' state and warning when anonymous is off without a token", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={true}
        anonymousEnabled={false}
        sessionExpirationSeconds={6 * 60 * 60}
      />,
    );

    expect(screen.getByText("Restricted")).toBeInTheDocument();
    expect(screen.getByText(/Anonymous access is off.*configure a token/)).toBeInTheDocument();
  });

  it("displays per-minute rate limit when configured", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={6 * 60 * 60}
        rateLimitPerMinute={45}
      />,
    );

    expect(screen.getByText(/45 requests per minute, per client IP/)).toBeInTheDocument();
    expect(screen.getByText(/FCP-only limiter namespace/)).toBeInTheDocument();
  });

  it("falls back to global cap message when no per-app rate limit", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={6 * 60 * 60}
      />,
    );

    expect(screen.getByText(/No per-app cap/)).toBeInTheDocument();
  });

  it("marks a custom handshake when one is configured", () => {
    render(
      <FcpSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/fcp"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={6 * 60 * 60}
        hasCustomHandshake={true}
      />,
    );

    expect(screen.getByText("Custom Markdown configured.")).toBeInTheDocument();
  });
});

describe("formatFcpSessionExpiration", () => {
  it("formats whole hours", () => {
    expect(formatFcpSessionExpiration(6 * 60 * 60)).toBe("6 hours");
    expect(formatFcpSessionExpiration(60 * 60)).toBe("1 hour");
  });

  it("formats fractional hours", () => {
    expect(formatFcpSessionExpiration(90 * 60)).toBe("1.5 hours");
  });

  it("formats minutes", () => {
    expect(formatFcpSessionExpiration(5 * 60)).toBe("5 minutes");
    expect(formatFcpSessionExpiration(60)).toBe("1 minute");
  });

  it("formats seconds for short values", () => {
    expect(formatFcpSessionExpiration(30)).toBe("30 seconds");
    expect(formatFcpSessionExpiration(1)).toBe("1 second");
  });

  it("returns 'Never' for zero or negative", () => {
    expect(formatFcpSessionExpiration(0)).toBe("Never");
    expect(formatFcpSessionExpiration(-1)).toBe("Never");
  });
});
