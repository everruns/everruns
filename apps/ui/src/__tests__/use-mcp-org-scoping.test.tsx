import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useMcpServerCatalog } from "@/hooks/use-mcp-servers";
import { useUserMcpConnections } from "@/hooks/use-user-connections";

const mockUseOrg = jest.fn();
const mockGetMcpServerCatalog = jest.fn();
const mockGetUserMcpConnections = jest.fn();

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => mockUseOrg(),
}));

jest.mock("@/lib/api/mcp-servers", () => ({
  getMcpServerCatalog: (...args: unknown[]) => mockGetMcpServerCatalog(...args),
  getMcpServerUsage: jest.fn(),
  mcpServersCrudApi: {
    create: jest.fn(),
    delete: jest.fn(),
    destroy: jest.fn(),
    get: jest.fn(),
    list: jest.fn(),
    update: jest.fn(),
  },
}));

jest.mock("@/lib/api/user-connections", () => ({
  createApiKeyConnection: jest.fn(),
  deleteUserConnection: jest.fn(),
  getConnectionProviders: jest.fn(),
  getUserConnections: jest.fn(),
  getUserMcpConnections: (...args: unknown[]) => mockGetUserMcpConnections(...args),
  verifyConnection: jest.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

describe("MCP organization-scoped queries", () => {
  let queryClient: QueryClient;
  let orgId: string | null;

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
    orgId = "org-old";
    mockUseOrg.mockImplementation(() => ({
      currentOrg: orgId ? { public_id: orgId } : null,
      isLoading: false,
    }));
    mockGetMcpServerCatalog.mockReset();
    mockGetUserMcpConnections.mockReset();
  });

  it("drops previous organization rows while the new organization loads", async () => {
    const nextCatalog = deferred<{
      data: Array<{ id: string; name: string }>;
      next_cursor: null;
    }>();
    const nextConnections = deferred<{
      data: Array<{ provider: string; server_name: string }>;
      next_cursor: null;
    }>();
    mockGetMcpServerCatalog
      .mockResolvedValueOnce({
        data: [{ id: "catalog-old", name: "Old catalog server" }],
        next_cursor: null,
      })
      .mockImplementationOnce(() => nextCatalog.promise);
    mockGetUserMcpConnections
      .mockResolvedValueOnce({
        data: [{ provider: "connection-old", server_name: "Old connection" }],
        next_cursor: null,
      })
      .mockImplementationOnce(() => nextConnections.promise);

    const { result, rerender } = renderHook(
      () => ({
        catalog: useMcpServerCatalog(),
        connections: useUserMcpConnections(),
      }),
      { wrapper },
    );

    await waitFor(() => {
      expect(result.current.catalog.data?.[0]?.name).toBe("Old catalog server");
      expect(result.current.connections.data?.[0]?.server_name).toBe("Old connection");
    });

    orgId = "org-new";
    rerender();

    await waitFor(() => {
      expect(mockGetMcpServerCatalog).toHaveBeenCalledTimes(2);
      expect(mockGetUserMcpConnections).toHaveBeenCalledTimes(2);
    });
    expect(result.current.catalog.data).toBeUndefined();
    expect(result.current.connections.data).toBeUndefined();

    act(() => {
      nextCatalog.resolve({
        data: [{ id: "catalog-new", name: "New catalog server" }],
        next_cursor: null,
      });
      nextConnections.resolve({
        data: [{ provider: "connection-new", server_name: "New connection" }],
        next_cursor: null,
      });
    });

    await waitFor(() => {
      expect(result.current.catalog.data?.[0]?.name).toBe("New catalog server");
      expect(result.current.connections.data?.[0]?.server_name).toBe("New connection");
    });
  });

  it("does not fetch either query before an organization is available", () => {
    orgId = null;

    const { result } = renderHook(
      () => ({
        catalog: useMcpServerCatalog(),
        connections: useUserMcpConnections(),
      }),
      { wrapper },
    );

    expect(result.current.catalog.data).toBeUndefined();
    expect(result.current.connections.data).toBeUndefined();
    expect(mockGetMcpServerCatalog).not.toHaveBeenCalled();
    expect(mockGetUserMcpConnections).not.toHaveBeenCalled();
  });
});
