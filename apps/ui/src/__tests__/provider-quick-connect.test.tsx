import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QuickConnect } from "@/app/(main)/settings/providers/quick-connect";
import type { Provider } from "@/lib/api/types";

const mockCreateProvider = jest.fn();
const mockCheckCredentials = jest.fn();

jest.mock("@/hooks/use-providers", () => ({
  useCreateProvider: () => ({ mutateAsync: mockCreateProvider, isPending: false }),
  useProvidersConfig: () => ({
    data: {
      policies: {},
      drivers: [
        { driver: "anthropic", supports_oauth: false, credential_schema: { fields: [] } },
        { driver: "openrouter", supports_oauth: true, credential_schema: { fields: [] } },
      ],
    },
    isLoading: false,
    error: null,
  }),
}));

jest.mock("@/lib/api/providers", () => ({
  checkProviderCredentials: (...args: unknown[]) => mockCheckCredentials(...args),
  providerSupportsOAuth: (driver: string) => driver === "openrouter",
  providerOAuthAuthorizeUrl: (id: string) => `/api/v1/providers/${id}/oauth/authorize`,
}));

const provider = (overrides: Partial<Provider>): Provider =>
  ({
    id: "provider-1",
    name: "Anthropic",
    provider_type: "anthropic",
    status: "active",
    api_key_set: true,
    managed: false,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) as Provider;

// The debounce in useCredentialCheck is 600ms; give verification assertions room.
const VERIFY_TIMEOUT = { timeout: 3000 };

describe("QuickConnect", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockCheckCredentials.mockResolvedValue({ status: "valid", models: 12 });
    mockCreateProvider.mockResolvedValue({ id: "provider-new" });
  });

  const renderGrid = (providers: Provider[] = [], onOpenAdvanced = jest.fn()) =>
    render(<QuickConnect providers={providers} onOpenAdvanced={onOpenAdvanced} />);

  it("offers one-click tiles for the key-only providers plus an escape hatch", () => {
    renderGrid();

    for (const name of ["Anthropic", "OpenAI", "Google Gemini", "OpenRouter", "Meta Model API"]) {
      expect(screen.getByText(name)).toBeInTheDocument();
    }
    expect(screen.getByText("Another provider")).toBeInTheDocument();
  });

  it("hands the full driver catalog to the advanced dialog", () => {
    const onOpenAdvanced = jest.fn();
    renderGrid([], onOpenAdvanced);

    fireEvent.click(screen.getByRole("button", { name: "Choose type" }));

    expect(onOpenAdvanced).toHaveBeenCalled();
  });

  it("verifies a pasted key without a button and creates a named provider", async () => {
    renderGrid();

    fireEvent.click(screen.getAllByRole("button", { name: /Connect/ })[0]);
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-ant-good" } });

    expect(await screen.findByText("Checking key…")).toBeInTheDocument();
    expect(
      await screen.findByText("Key valid — 12 models", {}, VERIFY_TIMEOUT),
    ).toBeInTheDocument();
    expect(mockCheckCredentials).toHaveBeenCalledWith({
      provider_type: "anthropic",
      credentials: { api_key: "sk-ant-good" },
      base_url: undefined,
    });

    fireEvent.click(screen.getByRole("button", { name: "Add Anthropic" }));

    // Name is derived from the tile, so the quick path asks for nothing but a key.
    await waitFor(() => {
      expect(mockCreateProvider).toHaveBeenCalledWith({
        name: "Anthropic",
        provider_type: "anthropic",
        api_key: "sk-ant-good",
      });
    });
  });

  it("surfaces a rejected key but still allows saving it", async () => {
    mockCheckCredentials.mockResolvedValue({ status: "rejected", message: "401 unauthorized" });
    renderGrid();

    fireEvent.click(screen.getAllByRole("button", { name: /Connect/ })[0]);
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-ant-bad" } });

    expect(await screen.findByText("Key rejected", {}, VERIFY_TIMEOUT)).toBeInTheDocument();
    const save = screen.getByRole("button", { name: "Add Anthropic anyway" });
    expect(save).toBeEnabled();

    fireEvent.click(save);
    await waitFor(() => expect(mockCreateProvider).toHaveBeenCalled());
  });

  it("keeps an unreachable check silent so it cannot block the user", async () => {
    mockCheckCredentials.mockResolvedValue({ status: "unreachable", message: "offline" });
    renderGrid();

    fireEvent.click(screen.getAllByRole("button", { name: /Connect/ })[0]);
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-ant-x" } });

    await waitFor(
      () => expect(screen.queryByText("Checking key…")).not.toBeInTheDocument(),
      VERIFY_TIMEOUT,
    );
    expect(screen.queryByText("Key rejected")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add Anthropic" })).toBeEnabled();
  });

  it("offers both key and sign-in for an OAuth-capable driver", async () => {
    renderGrid();

    expect(screen.getByText("Key or sign-in")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));

    // OAuth binds its state cookie to a provider id, so the provider is created
    // credential-free before the browser is handed to the authorize endpoint.
    await waitFor(() => {
      expect(mockCreateProvider).toHaveBeenCalledWith({
        name: "OpenRouter",
        provider_type: "openrouter",
      });
    });
  });

  it("deduplicates the derived name against providers already connected", async () => {
    renderGrid([provider({ name: "Anthropic" })]);

    expect(screen.getByText("1 connected")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "Add another" })[0]);
    expect(await screen.findByText("Anthropic 2")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-ant-two" } });
    fireEvent.click(screen.getByRole("button", { name: "Add Anthropic" }));

    await waitFor(() => {
      expect(mockCreateProvider).toHaveBeenCalledWith({
        name: "Anthropic 2",
        provider_type: "anthropic",
        api_key: "sk-ant-two",
      });
    });
  });
});
