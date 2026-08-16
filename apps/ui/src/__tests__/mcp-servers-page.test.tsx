import { render, screen, within, fireEvent, waitFor } from "@testing-library/react";
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
      reset: jest.fn(),
      isPending: false,
      error: null,
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

    expect(pageShell).toHaveClass("container", "mx-auto", "max-w-full", "p-4", "sm:p-6");
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

  it("surfaces a server-side create failure without closing the dialog", async () => {
    const mockMutateAsync = jest
      .fn()
      .mockRejectedValue(
        new Error("Invalid MCP server URL: Blocked host: 127.0.0.1 (private/internal address)"),
      );
    mockUseCreateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      isPending: false,
    });

    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Add Server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "local-mcp" } });
    fireEvent.change(within(dialog).getByLabelText("URL"), {
      target: { value: "http://127.0.0.1:9/mcp" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create Server" }));

    expect(
      await within(dialog).findByText(/Blocked host: 127\.0\.0\.1 \(private\/internal address\)/),
    ).toBeInTheDocument();
    expect(mockMutateAsync).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("opens the edit dialog prefilled with the server's current values", () => {
    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Edit MCP Server")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("Name")).toHaveValue("microsoft_learn");
    expect(within(dialog).getByLabelText("URL")).toHaveValue("https://learn.microsoft.com/api/mcp");
    expect(within(dialog).getByLabelText("Description (optional)")).toHaveValue(
      "Microsoft Learn documentation MCP server",
    );
  });

  it("submits updated name, description, and URL through the update hook", async () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: null,
    });

    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "updated-mcp-server" },
    });
    fireEvent.change(within(dialog).getByLabelText("Description (optional)"), {
      target: { value: "Updated description" },
    });
    fireEvent.change(within(dialog).getByLabelText("URL"), {
      target: { value: "https://new.mcp.com/v1/mcp" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mockMutateAsync).toHaveBeenCalledWith({
        name: "updated-mcp-server",
        description: "Updated description",
        url: "https://new.mcp.com/v1/mcp",
        protocol_mode: "auto",
      }),
    );
  });

  it("shows a validation error for invalid URLs when editing", async () => {
    const mockMutateAsync = jest.fn();
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: null,
    });

    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("URL"), { target: { value: "not-a-url" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    expect(await screen.findByText("URL must be a valid absolute URL")).toBeInTheDocument();
    expect(mockMutateAsync).not.toHaveBeenCalled();
  });

  it("surfaces an update failure in the edit dialog", async () => {
    const mockMutateAsync = jest.fn().mockRejectedValue(new Error("name already exists"));
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: new Error("name already exists"),
    });

    render(<McpServersPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    expect(await within(dialog).findByText(/name already exists/)).toBeInTheDocument();
  });
});
