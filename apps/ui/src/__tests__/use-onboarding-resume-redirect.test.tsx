import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import {
  isOnboardingRoute,
  useOnboardingResumeRedirect,
} from "@/components/onboarding/use-onboarding-resume-redirect";

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
  currentOrg: { public_id: "org_new" } as { public_id: string } | null,
  isLoading: false,
};
jest.mock("@/providers/auth-provider", () => ({ useAuth: () => authState }));
jest.mock("@/providers/org-provider", () => ({ useOrg: () => orgState }));

const mockGetOrganization = jest.fn();
jest.mock("@/lib/api/organizations", () => ({
  getOrganization: (org: string) => mockGetOrganization(org),
}));

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

describe("isOnboardingRoute", () => {
  it("matches the onboarding and setup routes", () => {
    expect(isOnboardingRoute("/onboarding")).toBe(true);
    expect(isOnboardingRoute("/orgs/org_abc/setup")).toBe(true);
  });
  it("does not match ordinary app routes", () => {
    expect(isOnboardingRoute("/dashboard")).toBe(false);
    expect(isOnboardingRoute("/orgs/org_abc/settings")).toBe(false);
    expect(isOnboardingRoute(null)).toBe(false);
  });
});

describe("useOnboardingResumeRedirect", () => {
  beforeEach(() => {
    mockReplace.mockClear();
    mockGetOrganization.mockReset();
    mockPathname.mockReturnValue("/dashboard");
    authState.isAuthenticated = true;
    authState.requiresAuth = true;
    authState.isLoading = false;
    orgState.currentOrg = { public_id: "org_new" };
    orgState.isLoading = false;
  });

  it("redirects to setup when the current org's onboarding is incomplete", async () => {
    mockGetOrganization.mockResolvedValue({ id: "org_new", onboarding_completed_at: null });
    const { result } = renderHook(() => useOnboardingResumeRedirect(), { wrapper });
    await waitFor(() =>
      expect(mockReplace).toHaveBeenCalledWith("/orgs/org_new/setup"),
    );
    expect(result.current).toBe(true);
  });

  it("does not redirect when onboarding is already complete (existing/backfilled org)", async () => {
    mockGetOrganization.mockResolvedValue({
      id: "org_new",
      onboarding_completed_at: "2024-01-01T00:00:00Z",
    });
    const { result } = renderHook(() => useOnboardingResumeRedirect(), { wrapper });
    // Let the query resolve, then assert no redirect happened.
    await waitFor(() => expect(mockGetOrganization).toHaveBeenCalled());
    expect(mockReplace).not.toHaveBeenCalled();
    expect(result.current).toBe(false);
  });

  it("does not redirect while already on the setup route (loop guard)", async () => {
    mockPathname.mockReturnValue("/orgs/org_new/setup");
    mockGetOrganization.mockResolvedValue({ id: "org_new", onboarding_completed_at: null });
    const { result } = renderHook(() => useOnboardingResumeRedirect(), { wrapper });
    expect(result.current).toBe(false);
    expect(mockReplace).not.toHaveBeenCalled();
    // The org query must not even run on the setup route.
    expect(mockGetOrganization).not.toHaveBeenCalled();
  });

  it("does not redirect while auth/org state is loading", () => {
    orgState.isLoading = true;
    const { result } = renderHook(() => useOnboardingResumeRedirect(), { wrapper });
    expect(result.current).toBe(false);
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("does not redirect when there is no current org (zero-org hook owns that case)", () => {
    orgState.currentOrg = null;
    const { result } = renderHook(() => useOnboardingResumeRedirect(), { wrapper });
    expect(result.current).toBe(false);
    expect(mockReplace).not.toHaveBeenCalled();
  });
});
