import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import PersonalAccessTokensPage from "@/app/(main)/settings/personal-access-tokens/page";

// Mock personal access token data
const mockTokens = [
  {
    id: "token-1",
    name: "Production Token",
    // API returns token_prefix already suffixed with an ellipsis (e.g. evr_pat_abc12345...)
    token_prefix: "evr_pat_prod_abc...",
    last_used_at: "2024-01-15T10:30:00Z",
    expires_at: "2025-01-01T00:00:00Z",
    created_at: "2024-01-01T00:00:00Z",
  },
  {
    id: "token-2",
    name: "Development Token",
    token_prefix: "evr_pat_dev_xyz...",
    last_used_at: null,
    expires_at: null,
    created_at: "2024-01-01T00:00:00Z",
  },
];

const mockUsePersonalAccessTokens = jest.fn();
const mockUseCreatePersonalAccessToken = jest.fn();
const mockUseDeletePersonalAccessToken = jest.fn();
const mockUseAuth = jest.fn();

jest.mock("@/hooks/use-auth", () => ({
  usePersonalAccessTokens: () => mockUsePersonalAccessTokens(),
  useCreatePersonalAccessToken: () => mockUseCreatePersonalAccessToken(),
  useDeletePersonalAccessToken: () => mockUseDeletePersonalAccessToken(),
}));

jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => mockUseAuth(),
}));

describe("PersonalAccessTokensPage", () => {
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

    // Default: auth required
    mockUseAuth.mockReturnValue({
      requiresAuth: true,
    });

    mockUsePersonalAccessTokens.mockReturnValue({
      data: mockTokens,
      isLoading: false,
      error: null,
    });

    mockUseCreatePersonalAccessToken.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseDeletePersonalAccessToken.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("renders Personal access tokens section header", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    expect(screen.getByText("Personal access tokens")).toBeInTheDocument();
    expect(
      screen.getByText(/Manage your personal access tokens for programmatic access/),
    ).toBeInTheDocument();
  });

  it("renders the account access warning as a reusable notice", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    const notice = screen.getByText("Full account access").closest('[data-slot="notice"]');

    expect(notice).toHaveAttribute("data-variant", "warning");
    expect(
      screen.getByText(/Personal access tokens are tied to your user account/),
    ).toBeInTheDocument();
  });

  it("renders token rows with correct data", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    expect(screen.getByText("Production Token")).toBeInTheDocument();
    expect(screen.getByText("Development Token")).toBeInTheDocument();
    expect(screen.getByText("evr_pat_prod_abc...")).toBeInTheDocument();
    expect(screen.getByText("evr_pat_dev_xyz...")).toBeInTheDocument();
  });

  it("shows expiration date for tokens with expiration", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    // Production token has expiration
    expect(screen.getByText(/Expires:/)).toBeInTheDocument();
  });

  it("shows last used date", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    // Production token was used on 2024-01-15
    expect(screen.getByText(/Last used: 1\/15\/2024/)).toBeInTheDocument();
    // Development token was never used
    expect(screen.getByText(/Last used: Never/)).toBeInTheDocument();
  });

  it("shows loading skeleton when tokens are loading", () => {
    mockUsePersonalAccessTokens.mockReturnValue({
      data: [],
      isLoading: true,
      error: null,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
    expect(skeletons.length).toBeGreaterThan(0);
  });

  it("shows empty state when no tokens exist", () => {
    mockUsePersonalAccessTokens.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    expect(screen.getByText("No personal access tokens")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Create a personal access token to access the Everruns API programmatically.",
      ),
    ).toBeInTheDocument();
  });

  it("shows error message when tokens fail to load", () => {
    mockUsePersonalAccessTokens.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    expect(screen.getByText(/Failed to load personal access tokens/)).toBeInTheDocument();
  });

  it("renders Create token button", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    const createButtons = screen.getAllByRole("button", { name: /Create token/i });
    expect(createButtons.length).toBeGreaterThan(0);
  });

  it("opens Create token dialog when clicking Create button", async () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    const createButton = screen.getAllByRole("button", { name: /Create token/i })[0];
    fireEvent.click(createButton);

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/The token inherits access to every organization/),
    ).toBeInTheDocument();
  });

  it("shows fixed expiration presets with 90 days selected by default", async () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");

    expect(within(dialog).getByRole("button", { name: "1 day" })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "30 days" })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "90 days" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      within(dialog).getByRole("button", { name: /Unrestricted \(not recommended\)/i }),
    ).toBeInTheDocument();
  });

  it("submits the default 90 day expiration when creating a token", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({ token: "evr_pat_secret_test" });
    mockUseCreatePersonalAccessToken.mockReturnValue({
      mutateAsync,
      isPending: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "Test Token" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: /^Create token$/i }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Test Token",
        expires_in_days: 90,
      });
    });
  });

  it("omits expiration when unrestricted is selected", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({ token: "evr_pat_secret_test" });
    mockUseCreatePersonalAccessToken.mockReturnValue({
      mutateAsync,
      isPending: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "Unlimited Token" },
    });
    fireEvent.click(
      within(dialog).getByRole("button", { name: /Unrestricted \(not recommended\)/i }),
    );
    fireEvent.click(within(dialog).getByRole("button", { name: /^Create token$/i }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Unlimited Token",
        expires_in_days: undefined,
      });
    });
  });

  it("shows authentication disabled message when auth is not required", () => {
    mockUseAuth.mockReturnValue({
      requiresAuth: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    expect(screen.getByText("Authentication Disabled")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Personal access tokens are only available when authentication is enabled. Contact your administrator to enable authentication.",
      ),
    ).toBeInTheDocument();
  });

  it("does not show Create button when auth is disabled", () => {
    mockUseAuth.mockReturnValue({
      requiresAuth: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    expect(screen.queryByRole("button", { name: /Create token/i })).not.toBeInTheDocument();
  });

  it("renders delete button for each token", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    // Should have 2 delete buttons (one for each token)
    const deleteButtons = document.querySelectorAll('button[class*="text-destructive"]');
    expect(deleteButtons.length).toBe(2);
  });
});
