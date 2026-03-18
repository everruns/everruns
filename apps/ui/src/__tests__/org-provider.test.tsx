import { renderHook, act } from "@testing-library/react";
import { ReactNode } from "react";
import { OrgProvider, useOrg } from "@/providers/org-provider";
import type { OrganizationMembership } from "@/lib/api/types";

// Mock next/navigation
const mockPush = jest.fn();
const mockPathname = jest.fn().mockReturnValue("/dashboard");
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn() }),
  usePathname: () => mockPathname(),
}));

// Mock switchOrg API
jest.mock("@/lib/api/users", () => ({
  switchOrg: jest.fn().mockResolvedValue(undefined),
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

function wrapper({ children }: { children: ReactNode }) {
  return <OrgProvider>{children}</OrgProvider>;
}

describe("OrgProvider", () => {
  it("initializes to default org", () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    expect(result.current.currentOrg?.public_id).toBe(DEFAULT_ORG.public_id);
  });

  it("switches to a different existing org", () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });

    expect(result.current.currentOrg?.public_id).toBe(SECOND_ORG.public_id);
    expect(storageMap.get("everruns_current_org")).toBe(SECOND_ORG.public_id);
  });

  it("keeps newly created org even when organizations list hasn't caught up", () => {
    // Start with only the default org
    mockUser = { organizations: [DEFAULT_ORG] };
    mockAuthLoading = false;

    const { result, rerender } = renderHook(() => useOrg(), { wrapper });

    // Simulate: user creates a new org and calls setCurrentOrg before
    // the organizations list is refreshed.
    act(() => {
      result.current.setCurrentOrg(NEW_ORG);
    });

    // The new org is NOT yet in the organizations array (query hasn't refetched).
    // Without the fix, the sync useEffect would reset currentOrg to default.
    expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);

    // Rerender to trigger effects again — should still hold the new org.
    rerender();
    expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);
  });

  it("clears explicit flag once organizations list catches up", () => {
    mockUser = { organizations: [DEFAULT_ORG] };
    mockAuthLoading = false;

    const { result, rerender } = renderHook(() => useOrg(), { wrapper });

    // Set current org to a new org not yet in the list
    act(() => {
      result.current.setCurrentOrg(NEW_ORG);
    });

    expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);

    // Now simulate the organizations list catching up (auth query refetched)
    mockUser = { organizations: [DEFAULT_ORG, NEW_ORG] };
    rerender();

    // Still the new org
    expect(result.current.currentOrg?.public_id).toBe(NEW_ORG.public_id);
  });

  it("redirects entity detail pages on org switch", () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockPathname.mockReturnValue("/agents/agent-123");

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
    });

    expect(mockPush).toHaveBeenCalledWith("/agents");
  });

  it("does not redirect non-entity pages on org switch", () => {
    mockUser = { organizations: [DEFAULT_ORG, SECOND_ORG] };
    mockAuthLoading = false;
    mockPathname.mockReturnValue("/dashboard");

    const { result } = renderHook(() => useOrg(), { wrapper });

    act(() => {
      result.current.setCurrentOrg(SECOND_ORG);
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
