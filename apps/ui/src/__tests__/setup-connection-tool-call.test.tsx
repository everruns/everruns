import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { SetupConnectionToolCall } from "@/components/chat/setup-connection-tool-call";
import type { ToolCompletedData } from "@/lib/api/types";

const submitToolResults = jest.fn();

jest.mock("@/lib/api/sessions", () => ({
  submitToolResults: (...args: unknown[]) => submitToolResults(...args),
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useConnectionProviders: () => ({
    data: [
      {
        provider_id: "mcp_oauth_linear",
        display_name: "Linear",
        icon: "link",
        connection_type: "oauth",
      },
      {
        provider_id: "daytona",
        display_name: "Daytona",
        icon: "link",
        connection_type: "api_key",
      },
    ],
  }),
}));

jest.mock("@/components/connections/api-key-dialog", () => ({
  ApiKeyDialog: ({ open }: { open: boolean }) =>
    open ? <div aria-label="Legacy connection dialog">Legacy connection dialog</div> : null,
}));

interface RenderCardOptions {
  provider?: string;
  subject?: "agent" | "user";
  setupUrl?: string;
}

function renderCard({ provider = "mcp_oauth_linear", subject, setupUrl }: RenderCardOptions = {}) {
  return render(
    <SetupConnectionToolCall
      sessionId="session_1"
      toolCallId="setup_1"
      provider={provider}
      subject={subject}
      setupUrl={setupUrl}
      toolResultsMap={new Map<string, ToolCompletedData>()}
    />,
  );
}

beforeEach(() => {
  submitToolResults.mockReset();
  submitToolResults.mockResolvedValue(undefined);
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe("SetupConnectionToolCall", () => {
  it("names the agent and opens its MCP setup route before continuing", async () => {
    const open = jest.spyOn(window, "open").mockImplementation(() => null);
    renderCard({
      subject: "agent",
      setupUrl: "/agents/agent_123?tab=mcp",
    });

    expect(screen.getByText("Linear connection required for this agent")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open agent MCP" }));

    expect(open).toHaveBeenCalledWith("/agents/agent_123?tab=mcp", "_blank", "noopener,noreferrer");
    expect(submitToolResults).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "I've connected — continue" }));
    await waitFor(() =>
      expect(submitToolResults).toHaveBeenCalledWith("session_1", [
        {
          tool_call_id: "setup_1",
          result: {
            connected: true,
            provider: "mcp_oauth_linear",
            subject: "agent",
          },
        },
      ]),
    );
  });

  it("names the user and opens their Connections page", () => {
    const open = jest.spyOn(window, "open").mockImplementation(() => null);
    renderCard({
      subject: "user",
      setupUrl: "/settings/connections",
    });

    expect(screen.getByText("Your Linear connection is required")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open connections" }));
    expect(open).toHaveBeenCalledWith("/settings/connections", "_blank", "noopener,noreferrer");
  });

  it("keeps the provider-only API key dialog flow", () => {
    const open = jest.spyOn(window, "open").mockImplementation(() => null);
    renderCard({ provider: "daytona" });

    expect(screen.getByText("Daytona connection required")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(screen.getByLabelText("Legacy connection dialog")).toBeInTheDocument();
    expect(open).not.toHaveBeenCalled();
  });

  it("does not open an unsafe setup URL", () => {
    const open = jest.spyOn(window, "open").mockImplementation(() => null);
    renderCard({
      subject: "user",
      setupUrl: "//attacker.example/connections",
    });

    expect(screen.getByText("The setup link is unavailable.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open connections" })).toBeDisabled();
    expect(open).not.toHaveBeenCalled();
  });
});
