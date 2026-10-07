import { fireEvent, render, screen } from "@testing-library/react";
import { UserMcpServersPanel } from "@/components/connections/user-mcp-servers-panel";

const mockUseUserMcpServers = jest.fn();
const mockUpdate = jest.fn();
const mockRemove = jest.fn();

jest.mock("@/hooks/use-user-mcp-servers", () => ({
  useUserMcpServers: () => mockUseUserMcpServers(),
  useAddUserMcpServer: () => ({ mutateAsync: jest.fn(), reset: jest.fn(), isPending: false }),
  useUpdateUserMcpServer: () => ({ mutate: mockUpdate, isPending: false, error: null }),
  useRemoveUserMcpServer: () => ({ mutate: mockRemove, isPending: false }),
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
  connection: { status: "not_connected", provider: "mcp_oauth_1" },
  created_at: "2026-10-06T00:00:00Z",
  updated_at: "2026-10-06T00:00:00Z",
  ...overrides,
});

describe("UserMcpServersPanel", () => {
  beforeEach(() => {
    mockUpdate.mockReset();
    mockRemove.mockReset();
  });

  it("explains the section when there are no servers", () => {
    mockUseUserMcpServers.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<UserMcpServersPanel />);
    expect(screen.getByText("My MCP servers")).toBeInTheDocument();
    expect(screen.getByText("No MCP servers yet")).toBeInTheDocument();
  });

  it("offers sign-in only to servers that still need it", () => {
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
    expect(screen.getAllByRole("button", { name: /Sign in/ })).toHaveLength(1);
  });

  it("turns a server off", () => {
    mockUseUserMcpServers.mockReturnValue({ data: [server({})], isLoading: false, error: null });
    render(<UserMcpServersPanel />);
    fireEvent.click(screen.getByRole("switch", { name: "Turn off notes" }));
    expect(mockUpdate).toHaveBeenCalledWith({ serverId: "mcp_1", request: { enabled: false } });
  });
});
