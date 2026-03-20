import { renderHook, act, waitFor } from "@testing-library/react";
import { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { OrgProvider, useOrg } from "@/providers/org-provider";
import type { OrganizationMembership } from "@/lib/api/types";

// Mock next/navigation
const mockPush = jest.fn();
const mockPathname = jest.fn().mockReturnValue("/dashboard");
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn() }),
  usePathname: () => mockPathname(),
}));

// Mock switchOrg API — controllable per test
const mockSwitchOrg = jest.fn().mockResolvedValue(undefined);
jest.mock("@/lib/api/users", () => ({
  switchOrg: (...args: unknown[]) => mockSwitchOrg(...args),
}));

// Mock auth provider — mutable so we can change the user mid-test
let mockUser: { organizations: OrganizationMembership[] } | null = null;
let mockAuthLoading = false;
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => ({ user: mockUser, isLoading: mockAuthLoading }),
}));

// localStorage mock
const storageMap = new Map<string, string>();
beforeEach(() => {
  storageMap.clear();
  jest.spyOn(Storage.prototype, "getItem").mockImplementation((key) => storageMap.get(key) ?? null);
  jest
    .spyOn(Storage.prototype, "setItem")
    .mockImplementation((key, val) => storageMap.set(key, val));
  jest.spyOn(Storage.prototype, "removeItem").mockImplementation((key) => storageMap.delete(key));
  mockPush.mockClear();
  mockSwitchOrg.mockClear().mockResolvedValue(undefined);
  mockPathname.mockReturnValue("/dashboard");
});

const DEFAULT_ORG: OrganizationMembership = {
  public_id: "org_00000000000000000000000000000001",
  name: "Default Org",
  role: "owner",
};

const SECOND_ORG: OrganizationMembership = {
  public_id: "org_second",
  name: "Second Org",
  role: "owner",
};

const NEW_ORG: OrganizationMembership = {
  public_id: "org_new",
  name: "Newly Created Org",
  role: "owner",
};

let queryClient: QueryClient;
beforeEach(() => {
  queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
});

function wrapper({ children }: { children: ReactNode }) {
  return (
    <QueryClientProvider client={queryClient}>
      <OrgProvider>{children}</OrgProvider>
    </QueryClientProvider>
  );
}

describe("OrgProvider", () => {
  it("initializes to default org", () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);
  });

  it("switches to a different existing org after cookie sync", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });

    // Org switch is async — wait for cookie sync to complete
    await waitFor(() => {
      expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id);
    });
    expect(storageMap.get("everruns_current_org")).toBe(SECOND_ORG.public_id);
    expect(mockSwitchOrg).toHaveBeenCalledWith(SECOND_ORG.public_id);
  });

  it("rolls back on failed org switch", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    // Reject only for SECOND_ORG; allow init sync for DEFAULT_ORG
    mockSwitchOrg.mockImplementation((orgId: string) => {
      if (orgId === SECOND_ORG.public_id) {
        return Promise.reject(new Error("network error"));
      }
      return Promise.resolve(undefined);
    });

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });

    // Wait for the failed switch to settle
    await waitFor(() => {
      expect(result.current.isSwitching).toBe(false);
    });

    // Should stay on original org
    expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);
    // localStorage should NOT have been updated
    expect(storageMap.get("everruns_current_org")).toBeUndefined();
  });

  it("shows isSwitching during org switch", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    // Make switchOrg hang for SECOND_ORG until we resolve it; allow init sync for DEFAULT_ORG
    let resolveSwitch!: () => void;
    mockSwitchOrg.mockImplementation((orgId: string) => {
      if (orgId === SECOND_ORG.public_id) {
        return new Promise<void>((resolve) => {
          resolveSwitch = resolve;
        });
      }
      return Promise.resolve(undefined);
    });

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });

    // Should be switching
    expect(result.current.isSwitching).toBe(true);
    // Should still be on old org until cookie sync completes
    expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);

    // Complete the switch
    await act(async () => {
      resolveSwitch();
    });

    expect(result.current.isSwitching).toBe(false);
    expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id);
  });

  it("keeps newly created org even when organizations list hasn't caught up", async () => {
    // Start with only the default org
    mockUser = { organizations: [DEFAULT_ORG] };
    mockAuthLoading = false;

    const { result, rerender } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(NEW_ORG);
    });

    // Wait for the async switch to complete
    await waitFor(() => {
      expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);
    });

    // Rerender to trigger effects again — should still hold the new org.
    rerender();
    expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);
  });

  it("clears explicit flag once organizations list catches up", async () => {
    mockUser = { organizations: [DEFAULT_ORG] };
    mockAuthLoading = false;

    const { result, rerender } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(NEW_ORG);
    });

    await waitFor(() => {
      expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);
    });

    // Now simulate the organizations list catching up (auth query refetched)
    mockUser = { organizations: [DEFAULT_ORG, NEW_ORG] };
    rerender();

    // Still the new org
    expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);
  });

  it("redirects entity detail pages on org switch", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockPathname.mockReturnValue("/agents/agent-123");

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });

    await waitFor(() => {
      expect(mockPush).toHaveBeenCalledWith("/agents");
    });
  });

  it("does not redirect non-entity pages on org switch", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockPathname.mockReturnValue("/dashboard");

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });

    await waitFor(() => {
      expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id);
    });

    expect(mockPush).not.toHaveBeenCalled();
  });

  it("restores org from localStorage on init", () => {
    storageMap.set("everruns_current_org", SECOND_ORG.public_id);
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id);
  });
});
