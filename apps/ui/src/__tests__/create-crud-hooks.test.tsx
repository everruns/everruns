import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { createCrudHooks } from "@/hooks/create-crud-hooks";

const mockUseOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => mockUseOrg(),
}));

describe("createCrudHooks", () => {
  let queryClient: QueryClient;

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });

    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org-123" },
      isLoading: false,
    });
  });

  it("adds org scope to list queries and passes includeArchived", async () => {
    const api = {
      create: jest.fn(),
      list: jest.fn().mockResolvedValue([{ id: "widget-1" }]),
      get: jest.fn(),
      update: jest.fn(),
      delete: jest.fn(),
      destroy: jest.fn(),
    };
    const hooks = createCrudHooks({
      api,
      queryKeys: {
        all: ["widgets"] as const,
        list: (includeArchived = false) => ["widgets", { includeArchived }] as const,
        detail: (id: string) => ["widget", id] as const,
      },
    });

    const { result } = renderHook(() => hooks.useList({ includeArchived: true }), { wrapper });

    expect(result.current.isLoading).toBe(true);
    await act(async () => {});

    expect(api.list).toHaveBeenCalledWith(true);
    expect(queryClient.getQueryData(["widgets", { includeArchived: true }, "org-123"])).toEqual([
      { id: "widget-1" },
    ]);
  });

  it("invalidates list and detail queries after update succeeds", async () => {
    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");
    const hooks = createCrudHooks({
      api: {
        create: jest.fn(),
        list: jest.fn(),
        get: jest.fn(),
        update: jest.fn().mockResolvedValue({ id: "widget-1", name: "Updated" }),
        delete: jest.fn(),
        destroy: jest.fn(),
      },
      queryKeys: {
        all: ["widgets"] as const,
        list: (includeArchived = false) => ["widgets", { includeArchived }] as const,
        detail: (id: string) => ["widget", id] as const,
      },
    });

    const { result } = renderHook(() => hooks.useUpdate(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ id: "widget-1", request: { archived: true } });
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["widgets"] });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["widget", "widget-1"] });
  });

  it.each(["archive", "destroy"] as const)("refreshes the open detail after %s", async (action) => {
    let status = "active";
    const api = {
      create: jest.fn(),
      list: jest.fn(),
      update: jest.fn(),
      get: jest.fn(async () => ({ id: "widget-1", status })),
      delete: jest.fn(async () => {
        status = "archived";
      }),
      destroy: jest.fn(async () => {
        status = "deleted";
      }),
    };
    const hooks = createCrudHooks({
      api,
      staleTime: 60_000,
      queryKeys: {
        all: ["widgets"],
        list: () => ["widgets"],
        detail: (id: string) => ["widget", id],
      },
    });
    const { result } = renderHook(
      () => ({
        detail: hooks.useDetail("widget-1"),
        mutation: action === "archive" ? hooks.useDelete() : hooks.useDestroy(),
      }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.detail.data?.status).toBe("active"));
    await act(async () => {
      await result.current.mutation.mutateAsync({ id: "widget-1" });
    });
    await waitFor(() =>
      expect(result.current.detail.data?.status).toBe(
        action === "archive" ? "archived" : "deleted",
      ),
    );
  });
});
