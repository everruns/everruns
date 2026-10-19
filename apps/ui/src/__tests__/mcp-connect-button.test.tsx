import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { McpConnectDialog } from "@/components/layout/mcp-connect-button";
import { codingAgentSetupPrompt } from "@/lib/coding-agent-setup";

const mockUseAuth = jest.fn();
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => mockUseAuth(),
}));

describe("McpConnectDialog", () => {
  it("explains that local no-auth connections require no authentication", () => {
    mockUseAuth.mockReturnValue({ requiresAuth: false });

    render(<McpConnectDialog open onOpenChange={jest.fn()} />);

    expect(screen.getByRole("dialog", { name: "Connect your coding agent" })).toHaveTextContent(
      "This Everruns deployment does not require authentication.",
    );
    expect(screen.queryByRole("link", { name: /Check connections/ })).not.toBeInTheDocument();
  });

  it("keeps a single setup prompt with recognizable agents", () => {
    mockUseAuth.mockReturnValue({ requiresAuth: true });

    render(<McpConnectDialog open onOpenChange={jest.fn()} />);

    expect(screen.getByRole("dialog", { name: "Connect your coding agent" })).toHaveTextContent(
      "Use the client's browser OAuth sign-in",
    );
    for (const name of ["Claude Code", "Codex", "Cursor", "Gemini CLI"]) {
      expect(screen.getByText(name)).toBeInTheDocument();
    }
    expect(screen.getByRole("link", { name: /Manual setup/ })).toBeInTheDocument();
    expect(screen.queryByText("MCP configuration")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /Check connections/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.queryByText(/Waiting for authorisation/)).not.toBeInTheDocument();
  });

  it("copies deployment-specific setup instructions", async () => {
    mockUseAuth.mockReturnValue({ requiresAuth: true });
    const writeText = jest.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const onOpenChange = jest.fn();
    render(<McpConnectDialog open onOpenChange={onOpenChange} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy setup prompt" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        codingAgentSetupPrompt(`${window.location.origin}/mcp`, true),
      ),
    );
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("offers manual copying when clipboard access fails", async () => {
    mockUseAuth.mockReturnValue({ requiresAuth: false });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: jest.fn().mockRejectedValue(new Error("Denied")) },
    });
    render(<McpConnectDialog open onOpenChange={jest.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy setup prompt" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Select and copy the text above");
  });

  it("keeps a self-hosted endpoint and no-auth instruction in the setup prompt", () => {
    const prompt = codingAgentSetupPrompt("http://localhost:9300/mcp", false);
    expect(prompt).toContain("http://localhost:9300/mcp");
    expect(prompt).toContain("does not require authentication");
    expect(prompt).not.toContain("app.everruns.com/mcp");
  });
});
