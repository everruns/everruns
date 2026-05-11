import { render, screen } from "@testing-library/react";
import {
  A2A_AGENT_CARD_PROTOCOL_VERSION,
  A2A_AGENT_CARD_VERSION,
  A2aAgentCardPreview,
  buildA2aAgentCardPreview,
} from "@/components/apps/a2a-agent-card-preview";

describe("A2aAgentCardPreview", () => {
  it("builds the public Agent Card shape without secret material", () => {
    const card = buildA2aAgentCardPreview({
      appName: "Support Bot",
      appDescription: "Routes support requests",
      endpointUrl: "https://example.com/api/v1/apps/app-123/a2a/appchan-123",
      agentCardName: "Support A2A",
      agentCardDescription: "Answers external agents",
      sessionMode: "session_per_invocation",
    });

    expect(card).toEqual({
      name: "Support A2A",
      description: "Answers external agents",
      url: "https://example.com/api/v1/apps/app-123/a2a/appchan-123",
      protocolVersion: A2A_AGENT_CARD_PROTOCOL_VERSION,
      version: A2A_AGENT_CARD_VERSION,
      preferredTransport: "JSONRPC",
      capabilities: {
        streaming: true,
        pushNotifications: false,
        stateTransitionHistory: false,
      },
      defaultInputModes: ["text/plain"],
      defaultOutputModes: ["text/plain"],
      skills: [
        {
          id: "default",
          name: "Support Bot",
          description: "Answers external agents",
          tags: ["everruns", "a2a"],
        },
      ],
      securitySchemes: {
        apiKey: { type: "http", scheme: "bearer" },
      },
      security: [{ apiKey: [] }],
    });
    expect(JSON.stringify(card)).not.toContain("api_key");
    expect(JSON.stringify(card)).not.toContain("secret");
  });

  it("redacts secret-shaped fields before rendering", () => {
    render(
      <A2aAgentCardPreview
        appName="Inbox Triage"
        sessionMode="shared_session"
        card={{
          name: "Inbox Triage",
          description: "Reviews inbox items",
          url: "https://example.com/a2a",
          protocolVersion: A2A_AGENT_CARD_PROTOCOL_VERSION,
          version: A2A_AGENT_CARD_VERSION,
          preferredTransport: "JSONRPC",
          capabilities: { streaming: false },
          skills: [{ id: "default", name: "Inbox Triage" }],
          securitySchemes: { apiKey: { type: "http", scheme: "bearer" } },
          api_key: "evra2a_0123456789abcdef0123456789abcdef",
          api_key_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        }}
      />,
    );

    const rendered = screen.getByLabelText("Agent Card JSON preview").textContent ?? "";
    expect(rendered).toContain("apiKey");
    expect(rendered).not.toContain("api_key");
    expect(rendered).not.toContain("api_key_hash");
    expect(rendered).not.toContain("0123456789abcdef0123456789abcdef");
    expect(rendered).not.toContain("evra2a_0123456789abcdef");
  });

  it("renders the inline JSON preview", () => {
    render(
      <A2aAgentCardPreview
        appName="Inbox Triage"
        appDescription="Reviews inbox items"
        endpointUrl="https://example.com/api/v1/apps/app-123/a2a/appchan-123"
        sessionMode="shared_session"
      />,
    );

    expect(screen.getByText("Agent Card preview")).toBeInTheDocument();
    expect(screen.getByText("Send only")).toBeInTheDocument();
    expect(screen.getByLabelText("Agent Card JSON preview")).toHaveTextContent(
      '"streaming": false',
    );
    expect(screen.getByLabelText("Agent Card JSON preview")).toHaveTextContent("Inbox Triage");
  });
});
