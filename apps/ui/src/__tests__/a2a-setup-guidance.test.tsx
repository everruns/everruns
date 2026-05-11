import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { A2aSetupGuidance } from "@/components/apps/a2a-setup-guidance";

const agentCard = {
  name: "Published A2A Agent",
  description: "Visible to external agents",
  url: "https://example.com/api/v1/apps/app-123/a2a/appchan-123",
  protocolVersion: "0.3.0",
  version: "0.1",
  preferredTransport: "JSONRPC",
  capabilities: {
    streaming: true,
    pushNotifications: false,
    stateTransitionHistory: false,
  },
  defaultInputModes: ["text/plain"],
  defaultOutputModes: ["text/plain"],
  skills: [{ id: "default", name: "Published A2A Agent", tags: ["everruns", "a2a"] }],
  securitySchemes: { apiKey: { type: "http", scheme: "bearer" } },
  security: [{ apiKey: [] }],
};

function renderGuidance(overrides: Partial<React.ComponentProps<typeof A2aSetupGuidance>> = {}) {
  return render(
    <A2aSetupGuidance
      endpointUrl="https://example.com/api/v1/apps/app-123/a2a/appchan-123"
      agentCardUrl="https://example.com/api/v1/apps/app-123/a2a/appchan-123/.well-known/agent-card.json"
      apiKeyPrefix="evra2a_12345678..."
      sessionMode="session_per_invocation"
      message="A2A request: {{a2a.text}}"
      appName="A2A App"
      isPublished={true}
      channelEnabled={true}
      {...overrides}
    />,
  );
}

describe("A2aSetupGuidance", () => {
  let originalFetch: typeof global.fetch;

  beforeEach(() => {
    originalFetch = global.fetch;
    global.fetch = jest.fn().mockResolvedValue({
      ok: true,
      json: async () => agentCard,
    }) as jest.Mock;
  });

  afterEach(() => {
    global.fetch = originalFetch;
    jest.restoreAllMocks();
  });

  it("fetches and renders the published Agent Card", async () => {
    renderGuidance();

    expect(global.fetch).toHaveBeenCalledWith(
      "https://example.com/api/v1/apps/app-123/a2a/appchan-123/.well-known/agent-card.json",
      expect.objectContaining({
        cache: "no-store",
        headers: { Accept: "application/json" },
      }),
    );
    expect((await screen.findAllByText("Published A2A Agent")).length).toBeGreaterThan(0);
    expect(screen.getByText("0.3.0")).toBeInTheDocument();
    expect(screen.getByText("JSONRPC")).toBeInTheDocument();
    expect(screen.getByText("streaming: true")).toBeInTheDocument();
    expect(screen.getByText("apiKey")).toBeInTheDocument();
  });

  it("refreshes the Agent Card on demand", async () => {
    renderGuidance();
    await screen.findAllByText("Published A2A Agent");

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    await waitFor(() => expect(global.fetch).toHaveBeenCalledTimes(2));
  });

  it("does not fetch while unpublished or disabled", () => {
    renderGuidance({ isPublished: false });
    expect(global.fetch).not.toHaveBeenCalled();
    expect(screen.getByText(/Publish the app and enable this channel/)).toBeInTheDocument();
  });

  it("keeps the raw URL copy fallback when fetch fails", async () => {
    (global.fetch as jest.Mock).mockResolvedValueOnce({ ok: false, status: 404 });
    renderGuidance();

    expect(await screen.findByText(/Could not load Agent Card/)).toBeInTheDocument();
    expect(
      screen.getAllByText(
        "https://example.com/api/v1/apps/app-123/a2a/appchan-123/.well-known/agent-card.json",
      ).length,
    ).toBeGreaterThan(0);
  });
});
