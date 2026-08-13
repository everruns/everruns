import { render, screen, waitFor } from "@testing-library/react";
import OnboardingLayout from "@/app/(onboarding)/layout";

const mockReplace = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: mockReplace }),
  usePathname: () => "/onboarding",
  useSearchParams: () => new URLSearchParams(),
}));

const authState = {
  isAuthenticated: true,
  isLoading: false,
  requiresAuth: true,
  authUnavailable: false,
};
const orgState = {
  organizations: [] as Array<{ public_id: string }>,
  isLoading: false,
};
jest.mock("@/providers/auth-provider", () => ({ useAuth: () => authState }));
jest.mock("@/providers/org-provider", () => ({ useOrg: () => orgState }));

describe("OnboardingLayout", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    authState.isAuthenticated = true;
    authState.isLoading = false;
    authState.requiresAuth = true;
    authState.authUnavailable = false;
    orgState.organizations = [];
    orgState.isLoading = false;
  });

  it("renders children for an authenticated zero-org user", () => {
    render(
      <OnboardingLayout>
        <div>Onboarding</div>
      </OnboardingLayout>,
    );
    expect(screen.getByText("Onboarding")).toBeInTheDocument();
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("shows the auth-unavailable state instead of a blank screen", () => {
    authState.authUnavailable = true;
    authState.isAuthenticated = false;

    render(
      <OnboardingLayout>
        <div>Onboarding</div>
      </OnboardingLayout>,
    );

    expect(screen.getByText("Authentication unavailable")).toBeInTheDocument();
    expect(screen.queryByText("Onboarding")).not.toBeInTheDocument();
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("redirects users who already have an org to the landing surface", async () => {
    orgState.organizations = [{ public_id: "org-1" }];

    render(
      <OnboardingLayout>
        <div>Onboarding</div>
      </OnboardingLayout>,
    );

    await waitFor(() => expect(mockReplace).toHaveBeenCalledWith("/chats"));
    expect(screen.queryByText("Onboarding")).not.toBeInTheDocument();
  });

  it("redirects unauthenticated users to login", async () => {
    authState.isAuthenticated = false;

    render(
      <OnboardingLayout>
        <div>Onboarding</div>
      </OnboardingLayout>,
    );

    await waitFor(() =>
      expect(mockReplace).toHaveBeenCalledWith(expect.stringContaining("/login")),
    );
    expect(screen.queryByText("Onboarding")).not.toBeInTheDocument();
  });
});
