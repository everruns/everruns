import { renderHook, waitFor } from "@testing-library/react";
import { ApiError } from "@/lib/api/client";
import { resolveOrgForResource } from "@/lib/api/resolver";
import { useResourceOrgFallback } from "@/hooks/use-resource-org-fallback";

const mockUseOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => mockUseOrg(),
}));

jest.mock("@/lib/api/resolver", () => ({
  resolveOrgForResource: jest.fn(),
}));

const mockResolveOrgForResource = resolveOrgForResource as jest.MockedFunction<
  typeof resolveOrgForResource
>;

describe("useResourceOrgFallback", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("switches to the owning organization and stays on the entity page after a detail 404", async () => {
    const setCurrentOrg = jest.fn();
    const targetOrg = { public_id: "org_target", name: "Target Org", role: "member" };

    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org_current", name: "Current Org", role: "member" },
      organizations: [{ public_id: "org_current", name: "Current Org", role: "member" }, targetOrg],
      setCurrentOrg,
      isSwitching: false,
    });
    mockResolveOrgForResource.mockResolvedValue({
      org_id: "org_target",
      org_name: "Target Org",
    });

    const { result } = renderHook(() =>
      useResourceOrgFallback({
        resourceId: "agent_00000000000000000000000000000042",
        error: new ApiError(404, "Not Found"),
        isLoading: false,
      }),
    );

    expect(result.current.isCheckingOtherOrgs).toBe(true);

    await waitFor(() => {
      expect(mockResolveOrgForResource).toHaveBeenCalledWith(
        "agent_00000000000000000000000000000042",
      );
      expect(setCurrentOrg).toHaveBeenCalledWith(targetOrg, { stayOnPage: true });
    });
  });

  it("does nothing when the resolved organization is not in the user's org list", async () => {
    const setCurrentOrg = jest.fn();

    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org_current", name: "Current Org", role: "member" },
      organizations: [{ public_id: "org_current", name: "Current Org", role: "member" }],
      setCurrentOrg,
      isSwitching: false,
    });
    mockResolveOrgForResource.mockResolvedValue({
      org_id: "org_missing",
      org_name: "Missing Org",
    });

    const { result } = renderHook(() =>
      useResourceOrgFallback({
        resourceId: "agent_00000000000000000000000000000042",
        error: new ApiError(404, "Not Found"),
        isLoading: false,
      }),
    );

    expect(result.current.isCheckingOtherOrgs).toBe(true);

    await waitFor(() => {
      expect(mockResolveOrgForResource).toHaveBeenCalledWith(
        "agent_00000000000000000000000000000042",
      );
      expect(result.current.isCheckingOtherOrgs).toBe(false);
    });
    expect(setCurrentOrg).not.toHaveBeenCalled();
  });
});
