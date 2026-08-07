import { render, screen } from "@testing-library/react";
import { McpConnectDialog } from "@/components/layout/mcp-connect-button";

const mockUseAuth = jest.fn();
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => mockUseAuth(),
}));

describe("McpConnectDialog", () => {
  it("explains that local no-auth connections require no authentication", () => {
    mockUseAuth.mockReturnValue({ requiresAuth: false });

    render(<McpConnectDialog open onOpenChange={jest.fn()} />);

    expect(screen.getByRole("dialog", { name: "Connect via MCP" })).toHaveTextContent(
      "This local Everruns server does not require authentication.",
    );
  });

  it("preserves OAuth guidance when authentication is required", () => {
    mockUseAuth.mockReturnValue({ requiresAuth: true });

    render(<McpConnectDialog open onOpenChange={jest.fn()} />);

    expect(screen.getByRole("dialog", { name: "Connect via MCP" })).toHaveTextContent(
      "Authentication is handled automatically via OAuth",
    );
  });
});
