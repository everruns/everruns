import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import PersonalAccessTokensPage, {
  defaultPersonalAccessTokenName,
} from "@/app/(main)/settings/personal-access-tokens/page";

// Mock personal access token data
const mockTokens = [
  {
    id: "token-1",
    name: "Production Token",
    // API returns token_prefix already suffixed with an ellipsis (e.g. evr_pat_abc12345...)
    token_prefix: "evr_pat_prod_abc...",
    scopes: ["*"],
    last_used_at: "2024-01-15T10:30:00Z",
    expires_at: "2025-01-01T00:00:00Z",
    created_at: "2024-01-01T00:00:00Z",
  },
  {
    id: "token-2",
    name: "Development Token",
    token_prefix: "evr_pat_dev_xyz...",
    scopes: ["read"],
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

  it("shows last used date", () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    // Production token was used on 2024-01-15
    expect(screen.getByText(/Last used: 1\/15\/2024/)).toBeInTheDocument();
    // Development token was never used
    expect(screen.getByText(/Last used: Never/)).toBeInTheDocument();
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

  it("puts an optional dated name at the bottom and submits it when left blank", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({ token: "evr_pat_secret_test" });
    mockUseCreatePersonalAccessToken.mockReturnValue({
      mutateAsync,
      isPending: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");
    const suggested = defaultPersonalAccessTokenName(new Date(), "en");
    const name = within(dialog).getByLabelText(/Name/);
    const access = within(dialog).getByRole("group", { name: "Access" });
    const expiration = within(dialog).getByRole("group", { name: "Expiration" });
    const submit = within(dialog).getByRole("button", { name: /^Create token$/i });

    expect(name).toHaveValue(suggested);
    expect(name).not.toBeRequired();
    expect(submit).toBeEnabled();
    const follows = Node.DOCUMENT_POSITION_FOLLOWING;
    expect(access.compareDocumentPosition(expiration) & follows).toBeTruthy();
    expect(expiration.compareDocumentPosition(name) & follows).toBeTruthy();
    expect(name.compareDocumentPosition(submit) & follows).toBeTruthy();

    fireEvent.change(name, { target: { value: "   " } });
    fireEvent.click(submit);

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        name: suggested,
        scopes: ["*"],
        expires_in_days: 90,
      });
    });
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
    fireEvent.change(within(dialog).getByLabelText(/Name/), {
      target: { value: "Test Token" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: /^Create token$/i }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Test Token",
        scopes: ["*"],
        expires_in_days: 90,
      });
    });
  });

  it("omits expiration when never expires is checked", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({ token: "evr_pat_secret_test" });
    mockUseCreatePersonalAccessToken.mockReturnValue({
      mutateAsync,
      isPending: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/Name/), {
      target: { value: "Unlimited Token" },
    });
    fireEvent.click(within(dialog).getByRole("checkbox", { name: /Never expires/i }));
    expect(within(dialog).getByRole("button", { name: "90 days" })).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("button", { name: /^Create token$/i }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Unlimited Token",
        scopes: ["*"],
        expires_in_days: undefined,
      });
    });
  });

  it("submits a custom number of days", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({ token: "evr_pat_secret_test" });
    mockUseCreatePersonalAccessToken.mockReturnValue({
      mutateAsync,
      isPending: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/Name/), {
      target: { value: "Custom Token" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Custom" }));

    const submit = within(dialog).getByRole("button", { name: /^Create token$/i });
    expect(submit).toBeDisabled();

    const days = within(dialog).getByLabelText("Days until expiration");
    fireEvent.change(days, { target: { value: "4000" } });
    expect(submit).toBeDisabled();
    expect(within(dialog).getByText(/from 1 to 3650/)).toBeInTheDocument();

    fireEvent.change(days, { target: { value: "45" } });
    expect(submit).toBeEnabled();
    fireEvent.click(submit);

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Custom Token",
        scopes: ["*"],
        expires_in_days: 45,
      });
    });
  });

  it("defaults the scope picker to full access and explains read-only reach", async () => {
    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("radio", { name: /Full access/ })).toBeChecked();
    const readOnly = within(dialog).getByRole("radio", { name: /Read-only/ });
    expect(readOnly).not.toBeChecked();
    expect(readOnly).toHaveAccessibleDescription(/Read access to all your organizations/);
    expect(within(dialog).queryByText(/safe to share/i)).not.toBeInTheDocument();
  });

  it("submits a read-only scope when Read-only is picked", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({ token: "evr_pat_secret_test" });
    mockUseCreatePersonalAccessToken.mockReturnValue({
      mutateAsync,
      isPending: false,
    });

    render(<PersonalAccessTokensPage />, { wrapper });

    fireEvent.click(screen.getAllByRole("button", { name: /Create token/i })[0]);

    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/Name/), {
      target: { value: "CI reader" },
    });
    fireEvent.click(within(dialog).getByRole("radio", { name: /Read-only/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: /^Create token$/i }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "CI reader",
        scopes: ["read"],
        expires_in_days: 90,
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
});

// Consumer action tests stub the decision boundary; dialog lifecycle is covered separately.
jest.mock("@/components/ui/confirmation-provider", () => ({
  useConfirmation: () => (message: string) => Promise.resolve(window.confirm(message)),
}));
