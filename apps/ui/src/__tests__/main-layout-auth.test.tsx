import { render, screen, waitFor } from "@testing-library/react";
import { ApiError } from "@/lib/api/client";
import MainLayout from "@/app/(main)/layout";

const mockReplace = jest.fn();
const mockPathname = jest.fn();
const mockSearchParams = jest.fn();

const authState = {
  config: { mode: "password", oauth_providers: [] },
  configLoading: false,
  configError: null as Error | null,
  user: { id: "user-1", email: "test@example.com", name: "Test User", roles: ["user"] },
  userLoading: false,
  userError: null as Error | null,
  isAuthenticated: true,
  requiresAuth: true,
  isLoading: false,
  authUnavailable: false,
  logout: jest.fn(),
  logoutPending: false,
};

const orgState = {
  isLoading: false,
};

jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: mockReplace }),
  usePathname: () => mockPathname(),
  useSearchParams: () => mockSearchParams(),
}));

jest.mock("@/components/layout/sidebar", () => ({
  Sidebar: () => <div data-testid="sidebar">Sidebar</div>,
}));

jest.mock("@/components/command-palette", () => ({
  CommandPalette: () => <div data-testid="command-palette">Command Palette</div>,
}));

jest.mock("@/hooks/use-command-palette", () => ({
  CommandPaletteContext: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useCommandPaletteState: () => ({}),
}));

jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => authState,
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => orgState,
}));

jest.mock("@/providers/notifications-provider", () => ({
  NotificationsProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => false,
}));

describe("MainLayout auth availability", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockPathname.mockReturnValue("/settings/providers");
    mockSearchParams.mockReturnValue(new URLSearchParams("tab=models"));

    authState.config = { mode: "password", oauth_providers: [] };
    authState.configLoading = false;
    authState.configError = null;
    authState.user = {
      id: "user-1",
      email: "test@example.com",
      name: "Test User",
      roles: ["user"],
    };
    authState.userLoading = false;
    authState.userError = null;
    authState.isAuthenticated = true;
    authState.requiresAuth = true;
    authState.isLoading = false;
    authState.authUnavailable = false;
    orgState.isLoading = false;
  });

  it("blocks the protected shell when auth config bootstrap fails", () => {
    authState.config = undefined;
    authState.configError = new Error("config failed");
    authState.user = null;
    authState.isAuthenticated = false;
    authState.requiresAuth = false;
    authState.authUnavailable = true;

    render(
      <MainLayout>
        <div>Secret App</div>
      </MainLayout>,
    );

    expect(screen.getByText("Authentication unavailable")).toBeInTheDocument();
    expect(screen.queryByTestId("sidebar")).not.toBeInTheDocument();
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("blocks the protected shell when current user bootstrap fails", () => {
    authState.user = null;
    authState.userError = new ApiError(503, "Service Unavailable", "backend down");
    authState.isAuthenticated = false;
    authState.authUnavailable = true;

    render(
      <MainLayout>
        <div>Secret App</div>
      </MainLayout>,
    );

    expect(screen.getByText("Authentication unavailable")).toBeInTheDocument();
    expect(screen.queryByTestId("sidebar")).not.toBeInTheDocument();
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("still redirects to login for ordinary unauthenticated 401 responses", async () => {
    authState.user = null;
    authState.userError = new ApiError(401, "Unauthorized", "not logged in");
    authState.isAuthenticated = false;
    authState.authUnavailable = false;

    render(
      <MainLayout>
        <div>Secret App</div>
      </MainLayout>,
    );

    await waitFor(() => {
      expect(mockReplace).toHaveBeenCalledWith(
        "/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels",
      );
    });
    expect(screen.queryByText("Authentication unavailable")).not.toBeInTheDocument();
  });
});
