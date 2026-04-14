import { render, screen, within, fireEvent } from "@testing-library/react";
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
          auth_mode: "api_key",
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

  it("shows confirmation dialog when clicking Archive", () => {
    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    expect(screen.getByText("Archive MCP Server")).toBeInTheDocument();
    expect(screen.getByText(/Are you sure you want to archive the MCP server/)).toBeInTheDocument();
  });

  it("does not archive when cancel is clicked in the confirmation dialog", () => {
    const mockMutateAsync = jest.fn();
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      isPending: false,
    });

    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    const dialog = screen.getByRole("dialog");
    const cancelButton = within(dialog).getByRole("button", { name: "Cancel" });
    fireEvent.click(cancelButton);

    expect(mockMutateAsync).not.toHaveBeenCalled();
  });

  it("archives server when confirmed in the dialog", () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      isPending: false,
    });

    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    const dialog = screen.getByRole("dialog");
    const archiveButton = within(dialog).getByRole("button", { name: "Archive" });
    fireEvent.click(archiveButton);

    expect(mockMutateAsync).toHaveBeenCalledWith({ status: "archived" });
  });

  it("shows a validation error for invalid MCP server URLs", async () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseCreateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      isPending: false,
    });

    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Add Server" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "atlassian" } });
    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "not-a-url" } });
    fireEvent.click(screen.getByRole("button", { name: "Create Server" }));

    expect(await screen.findByText("URL must be a valid absolute URL")).toBeInTheDocument();
    expect(mockMutateAsync).not.toHaveBeenCalled();
  });
});
