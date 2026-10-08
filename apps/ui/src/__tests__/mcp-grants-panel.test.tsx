import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { McpGrantsPanel } from "@/components/connections/mcp-grants-panel";

// The MCP page's "My connections" tab moved to Settings > My agent experience
// with the catalog's move to Settings (knowledge/integrations/user-mcp-servers.md, step 8).
const mockUseUserMcpConnections = jest.fn();
const mockUseDeleteUserConnection = jest.fn();

jest.mock("@/hooks/use-user-connections", () => ({
  useUserMcpConnections: () => mockUseUserMcpConnections(),
  useDeleteUserConnection: () => mockUseDeleteUserConnection(),
}));

describe("McpGrantsPanel", () => {
  beforeEach(() => {
    mockUseDeleteUserConnection.mockReturnValue({ mutate: jest.fn(), isPending: false });
  });

  it("renders the empty state", () => {
    mockUseUserMcpConnections.mockReturnValue({ data: [], isLoading: false, error: null });

    render(<McpGrantsPanel />);

    expect(screen.getByText("No MCP connections")).toBeInTheDocument();
  });

  it("loads the next personal-connections page through the continuation control", () => {
    const fetchNextPage = jest.fn();
    mockUseUserMcpConnections.mockReturnValue({
      data: [
        {
          provider: "mcp_oauth_11111111-1111-1111-1111-111111111111",
          server_id: "11111111-1111-1111-1111-111111111111",
          server_name: "linear",
          server_url: "https://mcp.linear.app/mcp",
          server_status: "active",
          provider_username: "person@example.com",
          scopes: "read write",
          connected_at: "2024-01-02T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
      hasNextPage: true,
      isFetchingNextPage: false,
      fetchNextPage,
    });

    render(<McpGrantsPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Load more connections" }));

    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("shows and revokes the current user's MCP connections", async () => {
    const revoke = jest.fn().mockResolvedValue({});
    mockUseUserMcpConnections.mockReturnValue({
      data: [
        {
          provider: "mcp_oauth_11111111-1111-1111-1111-111111111111",
          server_id: "11111111-1111-1111-1111-111111111111",
          server_name: "linear",
          server_url: "https://mcp.linear.app/mcp",
          server_status: "active",
          provider_username: "person@example.com",
          scopes: "read write",
          connected_at: "2024-01-02T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });
    mockUseDeleteUserConnection.mockReturnValue({
      mutate: revoke,
      isPending: false,
    });

    render(<McpGrantsPanel />);

    expect(screen.getByText("linear")).toBeInTheDocument();
    expect(screen.getByText("mcp.linear.app")).toBeInTheDocument();
    expect(screen.getByText("person@example.com")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
    await waitFor(() =>
      expect(revoke).toHaveBeenCalledWith("mcp_oauth_11111111-1111-1111-1111-111111111111"),
    );
  });

  it("keeps deleted presets visible and revocable", () => {
    mockUseUserMcpConnections.mockReturnValue({
      data: [
        {
          provider: "mcp_oauth_22222222-2222-2222-2222-222222222222",
          server_id: "22222222-2222-2222-2222-222222222222",
          server_name: "deleted-server",
          server_url: "https://deleted.example/mcp",
          server_status: "deleted",
          provider_username: null,
          scopes: null,
          connected_at: "2024-01-02T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<McpGrantsPanel />);

    expect(screen.getByText("Preset unavailable")).toBeInTheDocument();
    expect(screen.getByText("unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Revoke" })).toBeEnabled();
  });
});
