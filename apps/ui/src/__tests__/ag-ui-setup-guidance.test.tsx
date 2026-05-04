import { render, screen } from "@testing-library/react";
import { AgUiSetupGuidance, formatSessionExpiration } from "@/components/apps/ag-ui-setup-guidance";

describe("AgUiSetupGuidance", () => {
  it("renders the endpoint and anonymous status", () => {
    render(
      <AgUiSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/ag-ui"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={6 * 60 * 60}
      />,
    );

    expect(screen.getByText("Anonymous")).toBeInTheDocument();
    expect(screen.getByText("Ready for AG-UI clients")).toBeInTheDocument();
    expect(screen.getByText("https://example.com/api/v1/apps/app-123/ag-ui")).toBeInTheDocument();
    expect(screen.getByText("Responses stream back as AG-UI SSE events.")).toBeInTheDocument();
    expect(screen.getByText("Thread expiration")).toBeInTheDocument();
    expect(
      screen.getByText(/6 hours.*requests reusing the same threadId are rejected with 410 Gone/),
    ).toBeInTheDocument();
  });

  it("renders 'Never' when expiration is disabled", () => {
    render(
      <AgUiSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/ag-ui"
        isPublished={true}
        anonymousEnabled={true}
        sessionExpirationSeconds={0}
      />,
    );

    expect(screen.getByText(/Never.*threads can be resumed indefinitely/)).toBeInTheDocument();
  });
});

describe("formatSessionExpiration", () => {
  it("formats whole hours", () => {
    expect(formatSessionExpiration(6 * 60 * 60)).toBe("6 hours");
    expect(formatSessionExpiration(60 * 60)).toBe("1 hour");
  });

  it("formats fractional hours", () => {
    expect(formatSessionExpiration(90 * 60)).toBe("1.5 hours");
  });

  it("formats minutes", () => {
    expect(formatSessionExpiration(5 * 60)).toBe("5 minutes");
    expect(formatSessionExpiration(60)).toBe("1 minute");
  });

  it("formats seconds for short values", () => {
    expect(formatSessionExpiration(30)).toBe("30 seconds");
    expect(formatSessionExpiration(1)).toBe("1 second");
  });

  it("returns 'Never' for zero or negative", () => {
    expect(formatSessionExpiration(0)).toBe("Never");
    expect(formatSessionExpiration(-1)).toBe("Never");
  });

  it("displays rate limit when configured", () => {
    render(
      <AgUiSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/ag-ui"
        isPublished={true}
        anonymousEnabled={true}
        rateLimitPerMinute={120}
      />,
    );

    expect(screen.getByText("120 requests per minute, per IP")).toBeInTheDocument();
  });

  it("falls back to global cap message when rate limit is not configured", () => {
    render(
      <AgUiSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/ag-ui"
        isPublished={true}
        anonymousEnabled={true}
      />,
    );

    expect(screen.getByText("No per-app cap (global API limit applies)")).toBeInTheDocument();
  });
});
