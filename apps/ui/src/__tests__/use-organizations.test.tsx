import { renderHook, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { useCreateOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { authKeys } from "@/hooks/use-auth";
import { queryKeys } from "@/lib/query-keys";

jest.mock("@/lib/api/organizations", () => ({
  createOrganization: jest.fn(),
  getOrganization: jest.fn(),
  updateOrganization: jest.fn(),
}));

const mockUseOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => mockUseOrg(),
}));

import * as orgsApi from "@/lib/api/organizations";

const mockCreateOrganization = orgsApi.createOrganization as jest.MockedFunction<
  typeof orgsApi.createOrganization
>;
const mockUpdateOrganization = orgsApi.updateOrganization as jest.MockedFunction<
  typeof orgsApi.updateOrganization
>;

describe("useCreateOrganization", () => {
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
    jest.clearAllMocks();
    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org-123", name: "Existing Org", role: "owner" },
      isLoading: false,
    });
  });

  it("invalidates auth user query after creating org", async () => {
    const newOrg = { id: "org-456", name: "New Org" };
    mockCreateOrganization.mockResolvedValueOnce(newOrg as never);

    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreateOrganization(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ name: "New Org" });
    });

    // Must invalidate authKeys.user() so the sidebar org dropdown refreshes
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: authKeys.user() });
  });

  it("invalidates organizations query after creating org", async () => {
    const newOrg = { id: "org-456", name: "New Org" };
    mockCreateOrganization.mockResolvedValueOnce(newOrg as never);

    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreateOrganization(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ name: "New Org" });
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.organizations.all });
  });

  it("awaits invalidation before resolving mutateAsync", async () => {
    const newOrg = { id: "org-456", name: "New Org" };
    mockCreateOrganization.mockResolvedValueOnce(newOrg as never);

    // Track invalidation call order
    const callOrder: string[] = [];
    const originalInvalidate = queryClient.invalidateQueries.bind(queryClient);
    jest.spyOn(queryClient, "invalidateQueries").mockImplementation(async (opts) => {
      callOrder.push("invalidate");
      return originalInvalidate(opts);
    });

    const { result } = renderHook(() => useCreateOrganization(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ name: "New Org" });
      callOrder.push("resolved");
    });

    // Invalidation calls must happen before mutateAsync resolves
    const invalidateIndices = callOrder
      .map((c, i) => (c === "invalidate" ? i : -1))
      .filter((i) => i >= 0);
    const resolvedIndex = callOrder.indexOf("resolved");
    for (const idx of invalidateIndices) {
      expect(idx).toBeLessThan(resolvedIndex);
    }
  });
});

describe("useUpdateOrganization", () => {
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
    jest.clearAllMocks();
    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org-123", name: "Existing Org", role: "owner" },
      isLoading: false,
    });
  });

  it("invalidates auth user query after updating org", async () => {
    const updatedOrg = { id: "org-123", name: "Updated Org" };
    mockUpdateOrganization.mockResolvedValueOnce(updatedOrg as never);

    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUpdateOrganization(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ name: "Updated Org" });
    });

    // Must invalidate authKeys.user() so org dropdown reflects name changes
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: authKeys.user() });
  });

  it("invalidates org detail query after updating org", async () => {
    const updatedOrg = { id: "org-123", name: "Updated Org" };
    mockUpdateOrganization.mockResolvedValueOnce(updatedOrg as never);

    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUpdateOrganization(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ name: "Updated Org" });
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.organizations.detail("org-123"),
    });
  });
});
