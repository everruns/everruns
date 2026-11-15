import { renderHook, act, waitFor } from "@testing-library/react";
import { ReactNode } from "react";
import { QueryClient, QueryClientProvider, QueryObserver } from "@tanstack/react-query";
import { OrgProvider, entityIdFromPath, useOrg } from "@/providers/org-provider";
import { getActiveOrgId, setActiveOrgId } from "@/lib/api/active-org";
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

// Mock the cross-org resolver — null unless a test names an owning org
const mockResolveOrg = jest.fn().mockResolvedValue(null);
jest.mock("@/lib/api/resolver", () => ({
  resolveOrgForResource: (...args: unknown[]) => mockResolveOrg(...args),
}));

// Mock auth provider — mutable so we can change the user mid-test
let mockUser: { organizations: OrganizationMembership[] } | null = null;
let mockAuthLoading = false;
let mockInitialOrgId: string | null = null;
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
  mockResolveOrg.mockReset().mockResolvedValue(null);
  mockPathname.mockReturnValue("/dashboard");
  mockInitialOrgId = null;
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
      <OrgProvider initialOrgId={mockInitialOrgId}>{children}</OrgProvider>
    </QueryClientProvider>
  );
}

describe("OrgProvider", () => {
  it("publishes the org to the API layer as soon as it is chosen", async () => {
    // The cookie sync is a separate request that lands later; requests issued
    // before it must still name the right org, or their results land in the
    // wrong org's cache.
    setActiveOrgId(null);
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    storageMap.set("everruns_current_org", SECOND_ORG.public_id);

    renderHook(() => useOrg(), { wrapper });

    await waitFor(() => expect(getActiveOrgId()).toBe(SECOND_ORG.public_id));
  });

  it("initializes to default org", () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);
  });

  it("switches to a different existing org after cookie sync", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");

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
    expect(invalidateSpy).toHaveBeenCalledWith({ refetchType: "none" });
  });

  it("announces a switch made to stay on the page, not a user's own switch", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    const { result } = renderHook(() => useOrg(), { wrapper });
    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id));

    act(() => result.current.setCurrentOrg(SECOND_ORG));
    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id));
    expect(result.current.autoSwitch).toBeNull();

    act(() => result.current.setCurrentOrg(DEFAULT_ORG, { stayOnPage: true }));
    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id));
    expect(result.current.autoSwitch).toMatchObject({
      from: { public_id: SECOND_ORG.public_id },
      to: { public_id: DEFAULT_ORG.public_id },
    });
  });

  it("does not refetch the previous org's queries on switch", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    const { result } = renderHook(() => useOrg(), { wrapper });
    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id));

    // An active query in the first org: the switch must not re-send it.
    const fetches: string[] = [];
    const observer = new QueryObserver(queryClient, {
      queryKey: ["agents", "list"],
      queryFn: async () => {
        fetches.push(getActiveOrgId() ?? "none");
        return [];
      },
    });
    const unsubscribe = observer.subscribe(() => {});
    await waitFor(() => expect(fetches).toEqual([DEFAULT_ORG.public_id]));

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });
    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id));
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(fetches).toEqual([DEFAULT_ORG.public_id]);
    unsubscribe();
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

  it("syncs cookie when current org auto-falls back after org list refresh", async () => {
    storageMap.set("everruns_current_org", SECOND_ORG.public_id);
    mockInitialOrgId = SECOND_ORG.public_id;
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result, rerender } = renderHook(() => useOrg(), { wrapper });

    await waitFor(() => {
      expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id);
    });

    mockSwitchOrg.mockClear();

    // Simulate membership update removing current org; provider should
    // auto-fallback and resync cookie for the new org.
    mockUser = { organizations: [DEFAULT_ORG] };
    rerender();

    await waitFor(() => {
      expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);
    });

    expect(mockSwitchOrg).toHaveBeenCalledWith(DEFAULT_ORG.public_id);
  });

  it("restores org from localStorage on init", () => {
    storageMap.set("everruns_current_org", SECOND_ORG.public_id);
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id);
  });

  it("prefers request cookie org over localStorage during bootstrap", () => {
    storageMap.set("everruns_current_org", SECOND_ORG.public_id);
    mockInitialOrgId = DEFAULT_ORG.public_id;
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);
  });

  it("skips init cookie sync when request org already matches", () => {
    mockInitialOrgId = DEFAULT_ORG.public_id;
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    renderHook(() => useOrg(), { wrapper });

    expect(mockSwitchOrg).not.toHaveBeenCalled();
  });
});

it("refreshes the active organization's name and role without switching or resetting its queries", async () => {
  mockUser = { organizations: [DEFAULT_ORG] };
  mockAuthLoading = false;
  const { result, rerender } = renderHook(() => useOrg(), { wrapper });
  await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id));
  mockSwitchOrg.mockClear();
  const invalidate = jest.spyOn(queryClient, "invalidateQueries");
  mockUser = {
    organizations: [{ ...DEFAULT_ORG, name: "Northstar Studio", role: "member" }],
  };
  rerender();
  await waitFor(() => expect(result.current.currentOrg?.name).toBe("Northstar Studio"));
  expect(result.current.hasRole("admin")).toBe(false);
  expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);
  expect(mockSwitchOrg).not.toHaveBeenCalled();
  expect(invalidate).not.toHaveBeenCalled();
});

describe("OrgProvider landing on an entity link", () => {
  const SESSION = "session_01a1e096dc55722795cb947ff66ddff9";

  it("starts in the entity's owning org, never in the stored one", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockInitialOrgId = DEFAULT_ORG.public_id;
    mockPathname.mockReturnValue(`/sessions/${SESSION}/trace`);
    mockResolveOrg.mockResolvedValue({
      org_id: SECOND_ORG.public_id,
      org_name: "Second Org",
    });
    const committed: (string | undefined)[] = [];

    const { result } = renderHook(
      () => {
        const value = useOrg();
        committed.push(value.currentOrg?.public_id);
        return value;
      },
      { wrapper },
    );

    expect(result.current.isLoading).toBe(true);
    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id));
    expect(mockResolveOrg).toHaveBeenCalledWith(SESSION);
    // The cookie is moved before the commit, and nothing ever ran in the stored org.
    expect(mockSwitchOrg).toHaveBeenCalledWith(SECOND_ORG.public_id);
    expect(committed).not.toContain(DEFAULT_ORG.public_id);
    expect(mockPush).not.toHaveBeenCalled();
    expect(storageMap.get("everruns_current_org")).toBe(SECOND_ORG.public_id);
    // The switch is published for the shell to announce, with the way back.
    expect(result.current.autoSwitch).toMatchObject({
      id: 1,
      from: { public_id: DEFAULT_ORG.public_id },
      to: { public_id: SECOND_ORG.public_id },
    });
  });

  it("announces nothing when the link is in the org already selected", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockInitialOrgId = SECOND_ORG.public_id;
    mockPathname.mockReturnValue(`/sessions/${SESSION}`);
    mockResolveOrg.mockResolvedValue({ org_id: SECOND_ORG.public_id, org_name: "Second Org" });

    const { result } = renderHook(() => useOrg(), { wrapper });

    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id));
    expect(result.current.autoSwitch).toBeNull();
  });

  it("skips the cookie sync when the cookie already names the owning org", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockInitialOrgId = SECOND_ORG.public_id;
    mockPathname.mockReturnValue(`/sessions/${SESSION}`);
    mockResolveOrg.mockResolvedValue({
      org_id: SECOND_ORG.public_id,
      org_name: "Second Org",
    });

    const { result } = renderHook(() => useOrg(), { wrapper });

    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id));
    expect(mockSwitchOrg).not.toHaveBeenCalled();
  });

  it("falls back to the stored org when the owner is not one of the user's orgs", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockInitialOrgId = DEFAULT_ORG.public_id;
    mockPathname.mockReturnValue(`/sessions/${SESSION}`);
    mockResolveOrg.mockResolvedValue({
      org_id: "org_someone_else",
      org_name: "Other",
    });

    const { result } = renderHook(() => useOrg(), { wrapper });

    await waitFor(() => expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id));
    expect(mockSwitchOrg).not.toHaveBeenCalled();
  });

  it("does not resolve anything off entity routes", async () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockPathname.mockReturnValue("/sessions");

    const { result } = renderHook(() => useOrg(), { wrapper });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(mockResolveOrg).not.toHaveBeenCalled();
  });
});

describe("entityIdFromPath", () => {
  it("reads the prefixed id of an entity route", () => {
    expect(entityIdFromPath("/sessions/session_01a1e096dc55722795cb947ff66ddff9/trace")).toBe(
      "session_01a1e096dc55722795cb947ff66ddff9",
    );
    expect(entityIdFromPath("/agents/agent_0123456789abcdef0123456789abcdef")).toBe(
      "agent_0123456789abcdef0123456789abcdef",
    );
  });

  it("ignores list pages and non-id segments", () => {
    expect(entityIdFromPath("/sessions")).toBeNull();
    expect(entityIdFromPath("/agents/new")).toBeNull();
    expect(entityIdFromPath("/dashboard")).toBeNull();
  });
});
