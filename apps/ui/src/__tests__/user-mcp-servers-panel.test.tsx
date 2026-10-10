import { fireEvent, render, screen } from "@testing-library/react";
import { UserMcpServersPanel } from "@/components/connections/user-mcp-servers-panel";

const mockUseUserMcpServers = jest.fn();
const mockUseUserMcpConnections = jest.fn();
const mockUpdate = jest.fn();
const mockRemove = jest.fn();
const mockAdd = jest.fn();
const mockRevoke = jest.fn();

jest.mock("@/hooks/use-user-mcp-servers", () => ({
  useUserMcpServers: () => mockUseUserMcpServers(),
  useAddUserMcpServer: () => ({
    mutate: mockAdd,
    mutateAsync: jest.fn(),
    reset: jest.fn(),
    isPending: false,
    error: null,
  }),
  useUpdateUserMcpServer: () => ({ mutate: mockUpdate, isPending: false, error: null }),
  useRemoveUserMcpServer: () => ({ mutate: mockRemove, isPending: false }),
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useUserMcpConnections: () => mockUseUserMcpConnections(),
  useDeleteUserConnection: () => ({ mutate: mockRevoke, isPending: false }),
}));

jest.mock("@/hooks/use-mcp-servers", () => ({
  useMcpServerCatalog: () => ({ data: [], isLoading: false }),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => false, isLoading: false }),
}));

const server = (overrides: Record<string, unknown>) => ({
  id: "mcp_1",
  name: "notes",
  url: "https://notes.example.com/mcp",
  source: "custom",
  auth_mode: "oauth",
  enabled: true,
  deferred: true,
  connection: { status: "not_connected", provider: "mcp_oauth_1" },
  created_at: "2026-10-06T00:00:00Z",
  updated_at: "2026-10-06T00:00:00Z",
  ...overrides,
});

const signIn = (overrides: Record<string, unknown>) => ({
  provider: "mcp_oauth_1",
  server_id: "1",
  server_name: "linear",
  server_url: "https://mcp.linear.app/mcp",
  server_status: "active",
  connected_at: "2026-10-01T00:00:00Z",
  ...overrides,
});

describe("UserMcpServersPanel", () => {
  beforeEach(() => {
    mockUpdate.mockReset();
    mockRemove.mockReset();
    mockAdd.mockReset();
    mockRevoke.mockReset();
    mockUseUserMcpConnections.mockReturnValue({ data: [], isLoading: false, error: null });
  });

  it("explains the section when there are no servers", () => {
    mockUseUserMcpServers.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<UserMcpServersPanel />);
    expect(screen.getByText("My MCP servers")).toBeInTheDocument();
    expect(screen.getByText("No MCP servers yet")).toBeInTheDocument();
  });

  it("offers Connect to servers that need it and Reconnect to signed-in ones", () => {
    mockUseUserMcpServers.mockReturnValue({
      data: [
        server({}),
        server({
          id: "mcp_2",
          name: "linear",
          source: "catalog",
          connection: { status: "connected", provider: "mcp_oauth_2" },
        }),
      ],
      isLoading: false,
      error: null,
    });
    render(<UserMcpServersPanel />);
    expect(screen.getByText("Needs sign-in")).toBeInTheDocument();
    expect(screen.getByText("Signed in")).toBeInTheDocument();
    expect(screen.getByText("Catalog")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Connect$/ })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: /Reconnect/ })).toHaveLength(1);
  });

  it("turns a server off", () => {
    mockUseUserMcpServers.mockReturnValue({ data: [server({})], isLoading: false, error: null });
    render(<UserMcpServersPanel />);
    fireEvent.click(screen.getByRole("switch", { name: "Turn off notes" }));
    expect(mockUpdate).toHaveBeenCalledWith({ serverId: "mcp_1", request: { enabled: false } });
  });

  it("switches a server between loading tools on demand and always", () => {
    mockUseUserMcpServers.mockReturnValue({ data: [server({})], isLoading: false, error: null });
    const { rerender } = render(<UserMcpServersPanel />);
    const toggle = screen.getByRole("switch", { name: "Load tools on demand" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    expect(mockUpdate).toHaveBeenCalledWith({ serverId: "mcp_1", request: { deferred: false } });

    mockUseUserMcpServers.mockReturnValue({
      data: [server({ deferred: false })],
      isLoading: false,
      error: null,
    });
    rerender(<UserMcpServersPanel />);
    expect(screen.getByText(/listed at the start of every turn/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Load tools on demand" }));
    expect(mockUpdate).toHaveBeenLastCalledWith({
      serverId: "mcp_1",
      request: { deferred: true },
    });
  });

  it("shows one row per server, not a second one for its sign-in", () => {
    mockUseUserMcpServers.mockReturnValue({
      data: [
        server({
          id: "mcp_2",
          name: "visti",
          source: "catalog",
          catalog_name: "visti",
          connection: {
            status: "connected",
            provider: "mcp_oauth_22222222-2222-2222-2222-222222222222",
            connected_at: "2026-10-09T00:00:00Z",
          },
        }),
      ],
      isLoading: false,
      error: null,
    });
    mockUseUserMcpConnections.mockReturnValue({
      data: [
        signIn({
          provider: "mcp_oauth_22222222-2222-2222-2222-222222222222",
          server_name: "visti",
        }),
      ],
      isLoading: false,
      error: null,
    });
    render(<UserMcpServersPanel />);
    expect(screen.getAllByText("visti")).toHaveLength(1);
    expect(screen.queryByText("Sign-in only")).not.toBeInTheDocument();
    expect(screen.queryByText("MCP sign-ins for agent servers")).not.toBeInTheDocument();
  });

  it("keeps sign-ins that have no row visible, revocable and addable", () => {
    mockUseUserMcpServers.mockReturnValue({ data: [], isLoading: false, error: null });
    mockUseUserMcpConnections.mockReturnValue({
      data: [
        signIn({ provider: "mcp_oauth_1", server_name: "linear" }),
        signIn({ provider: "mcp_oauth_2", server_name: "gone", server_status: "deleted" }),
      ],
      isLoading: false,
      error: null,
    });
    render(<UserMcpServersPanel />);

    expect(screen.queryByText("No MCP servers yet")).not.toBeInTheDocument();
    expect(screen.getByText("Sign-in only")).toBeInTheDocument();
    expect(screen.getByText("Preset unavailable")).toBeInTheDocument();
    expect(screen.getByText("Server unavailable")).toBeInTheDocument();
    // Only the active catalog server can be added back to the list.
    fireEvent.click(screen.getByRole("button", { name: /Add to list/ }));
    expect(mockAdd).toHaveBeenCalledWith({ catalog: "linear" });
    const revoke = screen.getAllByRole("button", { name: "Revoke" });
    expect(revoke).toHaveLength(2);
    fireEvent.click(revoke[1]);
    expect(mockRevoke).toHaveBeenCalledWith("mcp_oauth_2");
  });

  it("loads more sign-ins", () => {
    const fetchNextPage = jest.fn();
    mockUseUserMcpServers.mockReturnValue({ data: [server({})], isLoading: false, error: null });
    mockUseUserMcpConnections.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
      hasNextPage: true,
      isFetchingNextPage: false,
      fetchNextPage,
    });
    render(<UserMcpServersPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Load more sign-ins" }));
    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("warns that removing a catalog server signs the person out of it", () => {
    const confirm = jest.spyOn(window, "confirm").mockReturnValue(true);
    mockUseUserMcpServers.mockReturnValue({
      data: [
        server({
          name: "visti",
          source: "catalog",
          connection: { status: "connected", provider: "mcp_oauth_3" },
        }),
      ],
      isLoading: false,
      error: null,
    });
    render(<UserMcpServersPanel />);
    fireEvent.click(screen.getByRole("button", { name: /Remove/ }));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("you are signed out of it"));
    expect(mockRemove).toHaveBeenCalledWith("mcp_1");
    confirm.mockRestore();
  });
});
