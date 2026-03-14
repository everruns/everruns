import { render, screen } from "@testing-library/react";
import McpServersPage from "@/app/(main)/mcp-servers/page";

const mockUseMcpServers = jest.fn();
const mockUseCreateMcpServer = jest.fn();
const mockUseUpdateMcpServer = jest.fn();
const mockUseDeleteMcpServer = jest.fn();
const mockUseDestroyMcpServer = jest.fn();

jest.mock("@/hooks/use-mcp-servers", () => ({
  useMcpServers: () => mockUseMcpServers(),
  useCreateMcpServer: () => mockUseCreateMcpServer(),
  useUpdateMcpServer: () => mockUseUpdateMcpServer(),
  useDeleteMcpServer: () => mockUseDeleteMcpServer(),
  useDestroyMcpServer: () => mockUseDestroyMcpServer(),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({
    can: () => true,
  }),
}));

describe("McpServersPage", () => {
  beforeEach(() => {
    mockUseMcpServers.mockReturnValue({
      data: [
        {
          id: "mcp-1",
          name: "microsoft_learn",
          description: "Microsoft Learn documentation MCP server",
          url: "https://learn.microsoft.com/api/mcp",
          transport_type: "http",
          status: "active",
          api_key_set: false,
          headers: {},
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    mockUseCreateMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseDeleteMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseDestroyMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("uses the standard main-page shell", () => {
    render(<McpServersPage />);

    const pageShell = screen
      .getByRole("heading", { level: 1, name: "MCP Servers" })
      .closest("div.container");

    expect(pageShell).toHaveClass("container", "mx-auto", "p-6");
  });

  it("renders configured MCP servers", () => {
    render(<McpServersPage />);

    expect(screen.getByText("microsoft_learn")).toBeInTheDocument();
    expect(screen.getByText("Microsoft Learn documentation MCP server")).toBeInTheDocument();
    expect(screen.getByText("Set Key")).toBeInTheDocument();
  });

  it("renders the empty state when no servers exist", () => {
    mockUseMcpServers.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<McpServersPage />);

    expect(screen.getByText("No MCP servers configured")).toBeInTheDocument();
  });

  it("renders the load error", () => {
    mockUseMcpServers.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<McpServersPage />);

    expect(screen.getByText(/Failed to load MCP servers/)).toBeInTheDocument();
  });
});
