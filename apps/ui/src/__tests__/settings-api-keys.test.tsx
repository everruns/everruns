import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ApiKeysPage from "@/app/(main)/settings/api-keys/page";

// Mock API keys data
const mockApiKeys = [
  {
    id: "key-1",
    name: "Production Key",
    key_prefix: "evr_prod_abc",
    last_used_at: "2024-01-15T10:30:00Z",
    expires_at: "2025-01-01T00:00:00Z",
    created_at: "2024-01-01T00:00:00Z",
  },
  {
    id: "key-2",
    name: "Development Key",
    key_prefix: "evr_dev_xyz",
    last_used_at: null,
    expires_at: null,
    created_at: "2024-01-01T00:00:00Z",
  },
];

const mockUseApiKeys = jest.fn();
const mockUseCreateApiKey = jest.fn();
const mockUseDeleteApiKey = jest.fn();
const mockUseAuth = jest.fn();

jest.mock("@/hooks/use-auth", () => ({
  useApiKeys: () => mockUseApiKeys(),
  useCreateApiKey: () => mockUseCreateApiKey(),
  useDeleteApiKey: () => mockUseDeleteApiKey(),
}));

jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => mockUseAuth(),
}));

describe("ApiKeysPage", () => {
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

    mockUseApiKeys.mockReturnValue({
      data: mockApiKeys,
      isLoading: false,
      error: null,
    });

    mockUseCreateApiKey.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseDeleteApiKey.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("renders API Keys section header", () => {
    render(<ApiKeysPage />, { wrapper });

    expect(screen.getByText("API Keys")).toBeInTheDocument();
    expect(
      screen.getByText("Manage your API keys for programmatic access.")
    ).toBeInTheDocument();
  });

  it("renders API key rows with correct data", () => {
    render(<ApiKeysPage />, { wrapper });

    expect(screen.getByText("Production Key")).toBeInTheDocument();
    expect(screen.getByText("Development Key")).toBeInTheDocument();
    expect(screen.getByText("evr_prod_abc...")).toBeInTheDocument();
    expect(screen.getByText("evr_dev_xyz...")).toBeInTheDocument();
  });

  it("shows expiration date for keys with expiration", () => {
    render(<ApiKeysPage />, { wrapper });

    // Production key has expiration
    expect(screen.getByText(/Expires:/)).toBeInTheDocument();
  });

  it("shows last used date", () => {
    render(<ApiKeysPage />, { wrapper });

    // Production key was used on 2024-01-15
    expect(screen.getByText(/Last used: 1\/15\/2024/)).toBeInTheDocument();
    // Development key was never used
    expect(screen.getByText(/Last used: Never/)).toBeInTheDocument();
  });

  it("shows loading skeleton when API keys are loading", () => {
    mockUseApiKeys.mockReturnValue({
      data: [],
      isLoading: true,
      error: null,
    });

    render(<ApiKeysPage />, { wrapper });

    const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
    expect(skeletons.length).toBeGreaterThan(0);
  });

  it("shows empty state when no API keys exist", () => {
    mockUseApiKeys.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<ApiKeysPage />, { wrapper });

    expect(screen.getByText("No API keys")).toBeInTheDocument();
    expect(
      screen.getByText("Create an API key to access the Everruns API programmatically.")
    ).toBeInTheDocument();
  });

  it("shows error message when API keys fail to load", () => {
    mockUseApiKeys.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<ApiKeysPage />, { wrapper });

    expect(screen.getByText(/Failed to load API keys/)).toBeInTheDocument();
  });

  it("renders Create API Key button", () => {
    render(<ApiKeysPage />, { wrapper });

    const createButtons = screen.getAllByRole("button", { name: /Create API Key/i });
    expect(createButtons.length).toBeGreaterThan(0);
  });

  it("opens Create API Key dialog when clicking Create button", async () => {
    render(<ApiKeysPage />, { wrapper });

    const createButton = screen.getAllByRole("button", { name: /Create API Key/i })[0];
    fireEvent.click(createButton);

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
      expect(screen.getByText("Create a new API key for programmatic access to the Everruns API.")).toBeInTheDocument();
    });
  });

  it("shows authentication disabled message when auth is not required", () => {
    mockUseAuth.mockReturnValue({
      requiresAuth: false,
    });

    render(<ApiKeysPage />, { wrapper });

    expect(screen.getByText("Authentication Disabled")).toBeInTheDocument();
    expect(
      screen.getByText("API keys are only available when authentication is enabled. Contact your administrator to enable authentication.")
    ).toBeInTheDocument();
  });

  it("does not show Create button when auth is disabled", () => {
    mockUseAuth.mockReturnValue({
      requiresAuth: false,
    });

    render(<ApiKeysPage />, { wrapper });

    expect(screen.queryByRole("button", { name: /Create API Key/i })).not.toBeInTheDocument();
  });

  it("renders delete button for each API key", () => {
    render(<ApiKeysPage />, { wrapper });

    // Should have 2 delete buttons (one for each key)
    const deleteButtons = document.querySelectorAll('button[class*="text-destructive"]');
    expect(deleteButtons.length).toBe(2);
  });
});
