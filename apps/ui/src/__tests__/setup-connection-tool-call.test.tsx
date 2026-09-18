import { render, screen } from "@testing-library/react";
import { SetupConnectionToolCall } from "@/components/chat/setup-connection-tool-call";
jest.mock("@/components/connections/api-key-dialog", () => ({
  ApiKeyDialog: () => null,
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useConnectionProviders: () => ({
    data: [
      {
        provider_id: "mcp_oauth_linear",
        display_name: "Linear",
        connection_type: "oauth",
        icon: "link",
      },
    ],
  }),
}));

jest.mock("@/lib/api/sessions", () => ({
  submitToolResults: jest.fn(),
}));

function renderCard(subject: { kind: "agent" | "user"; name: string }, setupUrl: string) {
  return render(
    <SetupConnectionToolCall
      sessionId="session_1"
      toolCallId="call_1"
      provider="mcp_oauth_linear"
      subject={subject}
      setupUrl={setupUrl}
      toolResultsMap={new Map()}
    />,
  );
}

describe("SetupConnectionToolCall", () => {
  it("names the agent and links service setup to its MCP tab", () => {
    renderCard({ kind: "agent", name: "Release Manager" }, "/agents/agent_1?tab=mcp");

    expect(
      screen.getByText("Connect Release Manager's Linear account to continue"),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open MCP settings/ })).toHaveAttribute(
      "href",
      "/agents/agent_1?tab=mcp",
    );
  });

  it("names the user and links user setup to connections", () => {
    renderCard({ kind: "user", name: "Ada Lovelace" }, "/settings/connections");

    expect(
      screen.getByText("Connect Ada Lovelace's Linear account to continue"),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open connections/ })).toHaveAttribute(
      "href",
      "/settings/connections",
    );
  });

  it("does not render an untrusted setup URL", () => {
    renderCard({ kind: "agent", name: "Release Manager" }, "javascript:alert(1)");

    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
    expect(screen.getByText("Connect your Linear account to continue")).toBeInTheDocument();
  });

  it("does not trust malformed subject metadata", () => {
    renderCard(
      { kind: "agent", name: 42 } as unknown as { kind: "agent"; name: string },
      "/agents/agent_1?tab=mcp",
    );

    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });
});
