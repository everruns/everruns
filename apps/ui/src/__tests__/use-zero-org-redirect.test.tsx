import { renderHook } from "@testing-library/react";
import { useZeroOrgRedirect } from "@/components/onboarding/use-zero-org-redirect";

const mockReplace = jest.fn();
const mockPathname = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: mockReplace }),
  usePathname: () => mockPathname(),
}));

const authState = {
  isAuthenticated: true,
  requiresAuth: true,
  isLoading: false,
};
const orgState = {
  organizations: [] as Array<{ public_id: string }>,
  isLoading: false,
};
jest.mock("@/providers/auth-provider", () => ({ useAuth: () => authState }));
jest.mock("@/providers/org-provider", () => ({ useOrg: () => orgState }));

describe("useZeroOrgRedirect", () => {
  beforeEach(() => {
    mockReplace.mockClear();
    mockPathname.mockReturnValue("/dashboard");
    authState.isAuthenticated = true;
    authState.requiresAuth = true;
    authState.isLoading = false;
    orgState.organizations = [];
    orgState.isLoading = false;
  });

  it("redirects authenticated zero-org users to onboarding", () => {
    const { result } = renderHook(() => useZeroOrgRedirect());
    expect(result.current).toBe(true);
    expect(mockReplace).toHaveBeenCalledWith("/onboarding");
  });

  it("does not redirect when the user has an org", () => {
    orgState.organizations = [{ public_id: "org-1" }];
    const { result } = renderHook(() => useZeroOrgRedirect());
    expect(result.current).toBe(false);
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("does not redirect while auth or org state is loading", () => {
    orgState.isLoading = true;
    const { result } = renderHook(() => useZeroOrgRedirect());
    expect(result.current).toBe(false);
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("does not redirect when auth is not required (OSS none mode)", () => {
    authState.requiresAuth = false;
    const { result } = renderHook(() => useZeroOrgRedirect());
    expect(result.current).toBe(false);
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("does not redirect when already on the onboarding route", () => {
    mockPathname.mockReturnValue("/onboarding");
    const { result } = renderHook(() => useZeroOrgRedirect());
    expect(result.current).toBe(false);
    expect(mockReplace).not.toHaveBeenCalled();
  });
});
