import { render, screen, within, fireEvent, waitFor } from "@testing-library/react";
import McpCatalogPage from "@/app/(main)/settings/mcp-catalog/page";

const mockUseMcpServerCatalog = jest.fn();
const mockUseMcpServerUsage = jest.fn();
const mockUseCreateMcpServer = jest.fn();
const mockUseDeleteMcpServer = jest.fn();
const mockUseUpdateMcpServer = jest.fn();
const mockUseDestroyMcpServer = jest.fn();
const mockUsePolicies = jest.fn();

jest.mock("@/hooks/use-mcp-servers", () => ({
  useMcpServerCatalog: () => mockUseMcpServerCatalog(),
  useMcpServerUsage: () => mockUseMcpServerUsage(),
  useCreateMcpServer: () => mockUseCreateMcpServer(),
  useDeleteMcpServer: () => mockUseDeleteMcpServer(),
  useUpdateMcpServer: () => mockUseUpdateMcpServer(),
  useDestroyMcpServer: () => mockUseDestroyMcpServer(),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => mockUsePolicies(),
}));

describe("McpCatalogPage", () => {
  beforeEach(() => {
    mockUsePolicies.mockReturnValue({
      isLoading: false,
      can: () => true,
    });

    mockUseMcpServerCatalog.mockReturnValue({
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
          used_by_agents: 2,
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });
    mockUseMcpServerUsage.mockReturnValue({
      data: {
        total_count: 2,
        agent_names: ["Docs agent", "Research agent"],
        truncated: false,
      },
      isLoading: false,
      error: null,
    });

    mockUseCreateMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseDeleteMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      reset: jest.fn(),
      isPending: false,
      error: null,
    });

    mockUseDestroyMcpServer.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("blocks archiving when agent usage cannot be loaded", () => {
    mockUseMcpServerUsage.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<McpCatalogPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText(/Archiving is blocked/)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Archive" })).toBeDisabled();
  });

  it("renders as a Settings > Organization page that explains presets", () => {
    render(<McpCatalogPage />);

    expect(screen.getByRole("heading", { level: 2, name: "MCP catalog" })).toBeInTheDocument();
    expect(
      screen.getByText("A preset does nothing until an agent or a person adds it."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "My connections" })).not.toBeInTheDocument();
    expect(screen.getByText("Used by")).toBeInTheDocument();
    expect(screen.getByText("2 agents")).toBeInTheDocument();
  });

  it("hides the catalog and its controls without view permission", () => {
    mockUsePolicies.mockReturnValue({
      isLoading: false,
      can: () => false,
    });

    render(<McpCatalogPage />);

    expect(screen.getByText("You cannot view the MCP catalog")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open My agent experience" })).toHaveAttribute(
      "href",
      "/settings/agent-experience",
    );
    expect(screen.queryByRole("button", { name: "Add Server" })).not.toBeInTheDocument();
    expect(screen.queryByText("microsoft_learn")).not.toBeInTheDocument();
  });

  it("shows the catalog read-only to people who can view but not manage it", () => {
    mockUsePolicies.mockReturnValue({
      isLoading: false,
      can: (policy: string) => policy === "mcp_server.view",
    });

    render(<McpCatalogPage />);

    expect(screen.getByText("microsoft_learn")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add Server" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Archive/ })).not.toBeInTheDocument();
  });

  it("renders configured MCP servers", () => {
    render(<McpCatalogPage />);

    expect(screen.getByText("microsoft_learn")).toBeInTheDocument();
    expect(screen.getByText("Microsoft Learn documentation MCP server")).toBeInTheDocument();
    expect(screen.getByText("Set Key")).toBeInTheDocument();
    expect(screen.getByText("2 agents")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Host" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Used by" })).toBeInTheDocument();
  });

  it("renders the empty state when no servers exist", () => {
    mockUseMcpServerCatalog.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<McpCatalogPage />);

    expect(screen.getByText("No MCP presets")).toBeInTheDocument();
  });

  it("renders the load error", () => {
    mockUseMcpServerCatalog.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<McpCatalogPage />);

    expect(screen.getByText(/Failed to load MCP catalog/)).toBeInTheDocument();
  });

  it("shows confirmation dialog when clicking Archive", () => {
    render(<McpCatalogPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    expect(screen.getByText("Archive MCP Server")).toBeInTheDocument();
    expect(screen.getByText(/Are you sure you want to archive the MCP server/)).toBeInTheDocument();
    expect(screen.getByText(/used by 2 active agents/)).toBeInTheDocument();
    expect(screen.getByText("Docs agent")).toBeInTheDocument();
    expect(screen.getByText("Research agent")).toBeInTheDocument();
  });

  it("does not archive when cancel is clicked in the confirmation dialog", () => {
    const mockMutateAsync = jest.fn();
    mockUseDeleteMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      isPending: false,
    });

    render(<McpCatalogPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    const dialog = screen.getByRole("dialog");
    const cancelButton = within(dialog).getByRole("button", { name: "Cancel" });
    fireEvent.click(cancelButton);

    expect(mockMutateAsync).not.toHaveBeenCalled();
  });

  it("archives server when confirmed in the dialog", async () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseDeleteMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      isPending: false,
    });

    render(<McpCatalogPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    const dialog = screen.getByRole("dialog");
    const archiveButton = within(dialog).getByRole("button", { name: "Archive" });
    fireEvent.click(archiveButton);

    await waitFor(() => expect(mockMutateAsync).toHaveBeenCalledWith("mcp-1"));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("loads the next catalog page through the continuation control", () => {
    const fetchNextPage = jest.fn();
    mockUseMcpServerCatalog.mockReturnValue({
      ...mockUseMcpServerCatalog(),
      hasNextPage: true,
      isFetchingNextPage: false,
      fetchNextPage,
    });

    render(<McpCatalogPage />);
    fireEvent.click(screen.getByRole("button", { name: "Load more presets" }));

    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("shows a validation error for invalid MCP server URLs", async () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseCreateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      isPending: false,
    });

    render(<McpCatalogPage />);

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

    render(<McpCatalogPage />);

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
    render(<McpCatalogPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Edit MCP Server")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("Name")).toHaveValue("microsoft_learn");
    expect(within(dialog).getByLabelText("URL")).toHaveValue("https://learn.microsoft.com/api/mcp");
    expect(within(dialog).getByLabelText("Description (optional)")).toHaveValue(
      "Microsoft Learn documentation MCP server",
    );
  });

  it("opens the edit dialog from the catalog row keyboard action", () => {
    render(<McpCatalogPage />);

    const row = screen.getByText("microsoft_learn").closest("tr");
    expect(row).not.toBeNull();
    fireEvent.keyDown(row!, { key: "Enter" });

    expect(screen.getByRole("dialog")).toHaveTextContent("Edit MCP Server");
  });

  it("submits updated name, description, and URL through the update hook", async () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: null,
    });

    render(<McpCatalogPage />);

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
        elicitation_policy: "url",
        service_connection_provider: "",
      }),
    );
  });

  it("keeps a preset's connection-backed agent credential when editing", async () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: null,
    });
    mockUseMcpServerCatalog.mockReturnValue({
      data: [
        {
          id: "mcp-gh",
          name: "github",
          description: "GitHub MCP server",
          url: "https://api.githubcopilot.com/mcp/",
          transport_type: "http",
          status: "active",
          auth_mode: "oauth",
          service_connection_provider: "github",
          api_key_set: false,
          headers: {},
          used_by_agents: 0,
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<McpCatalogPage />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByLabelText("Agent credential")).toHaveTextContent(
      "The agent's GitHub App",
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mockMutateAsync).toHaveBeenCalledWith(
        expect.objectContaining({ service_connection_provider: "github" }),
      ),
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

    render(<McpCatalogPage />);

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

    render(<McpCatalogPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    expect(await within(dialog).findByText(/name already exists/)).toBeInTheDocument();
  });

  it("requires a fresh API key and drops headers when moving a credentialed server to a new origin", async () => {
    const mockMutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMcpServer.mockReturnValue({
      mutateAsync: mockMutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: null,
    });
    mockUseMcpServerCatalog.mockReturnValue({
      data: [
        {
          id: "mcp-1",
          name: "microsoft_learn",
          description: null,
          url: "https://learn.microsoft.com/api/mcp",
          transport_type: "http",
          status: "active",
          auth_mode: "api_key",
          api_key_set: true,
          headers: { "X-Tenant": "***" },
          used_by_agents: 0,
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<McpCatalogPage />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");

    // Same-origin path change: no credential prompt.
    fireEvent.change(within(dialog).getByLabelText("URL"), {
      target: { value: "https://learn.microsoft.com/api/v2/mcp" },
    });
    expect(within(dialog).queryByLabelText("API Key")).not.toBeInTheDocument();

    fireEvent.change(within(dialog).getByLabelText("URL"), {
      target: { value: "https://other.example/mcp" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(
      await within(dialog).findByText("Re-enter the API key to move this server to a new origin"),
    ).toBeInTheDocument();
    expect(mockMutateAsync).not.toHaveBeenCalled();

    fireEvent.change(within(dialog).getByLabelText("API Key"), {
      target: { value: " sk-new " },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mockMutateAsync).toHaveBeenCalledWith(
        expect.objectContaining({
          url: "https://other.example/mcp",
          api_key: "sk-new",
          headers: {},
        }),
      ),
    );
  });
});
