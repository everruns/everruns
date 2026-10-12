import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import {
  ConnectProviderSheet,
  isProviderNameTaken,
  uniqueProviderName,
} from "@/components/providers/connect-provider-sheet";
import type { ModelWithProvider, Provider } from "@/lib/api/types";

const mockCreateProvider = jest.fn();
const mockSetModelsEnabled = jest.fn();
const mockReview = jest.fn();
const mockRouterPush = jest.fn();
const mockStartChatGptLogin = jest.fn();
const mockCheckCredentials = jest.fn();
let mockModels: ModelWithProvider[] = [];
let mockChatGptEnabled = false;

jest.mock("next/navigation", () => ({ useRouter: () => ({ push: mockRouterPush }) }));
jest.mock("@/lib/api/chatgpt", () => ({
  startChatGptLogin: (...args: unknown[]) => mockStartChatGptLogin(...args),
}));

const keyDriver = (driver: string, supportsOAuth = false) => ({
  driver,
  supports_oauth: supportsOAuth,
  credential_schema: {
    fields: [
      { name: "api_key", label: "API key", field_type: "password", required: !supportsOAuth },
    ],
    instructions_markdown: "",
  },
});

const mockConfig = {
  data: {
    policies: {},
    drivers: [keyDriver("anthropic"), keyDriver("openai"), keyDriver("openrouter", true)],
  },
  isLoading: false,
  error: null,
};
const mockConfigWithChatGpt = {
  ...mockConfig,
  data: {
    ...mockConfig.data,
    drivers: [
      { driver: "chatgpt", supports_oauth: true, credential_schema: { fields: [] } },
      ...mockConfig.data.drivers,
    ],
  },
};

jest.mock("@/hooks/use-providers", () => ({
  useCreateProvider: () => ({ mutateAsync: mockCreateProvider, isPending: false }),
  useModels: () => ({ data: mockModels, isFetching: false }),
  useSetModelsEnabled: () => ({ mutateAsync: mockSetModelsEnabled, isPending: false }),
  useReviewProviderModels: () => ({ mutateAsync: mockReview, isPending: false }),
  // Stable per mode, as react-query would return: a fresh object every render
  // would re-run effects keyed on the schema forever.
  useProvidersConfig: () => (mockChatGptEnabled ? mockConfigWithChatGpt : mockConfig),
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

const model = (overrides: Partial<ModelWithProvider>): ModelWithProvider =>
  ({
    id: "model-1",
    model_id: "claude-x",
    display_name: "Claude X",
    provider_id: "provider-new",
    provider_name: "Anthropic",
    provider_type: "anthropic",
    healthy: true,
    enabled: false,
    capabilities: ["chat"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) as ModelWithProvider;

const VERIFY_TIMEOUT = { timeout: 3000 };

describe("provider naming", () => {
  it("derives a free name and compares names without case or spaces", () => {
    expect(uniqueProviderName("Anthropic", [])).toBe("Anthropic");
    expect(uniqueProviderName("Anthropic", ["anthropic", "Anthropic 2"])).toBe("Anthropic 3");
    expect(isProviderNameTaken("  ANTHROPIC ", [provider({})])).toBe(true);
    expect(isProviderNameTaken("Anthropic EU", [provider({})])).toBe(false);
  });
});

describe("ConnectProviderSheet", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockModels = [];
    mockChatGptEnabled = false;
    mockCheckCredentials.mockResolvedValue({ status: "valid", models: 2 });
    mockCreateProvider.mockResolvedValue(provider({ id: "provider-new" }));
    mockSetModelsEnabled.mockResolvedValue({ failed: [] });
    mockReview.mockResolvedValue(undefined);
  });

  const renderSheet = (providers: Provider[] = [], onConnected = jest.fn()) =>
    render(
      <ConnectProviderSheet
        open
        onOpenChange={jest.fn()}
        providers={providers}
        onConnected={onConnected}
      />,
    );

  it("lists drivers and hides ChatGPT until the server offers it", () => {
    renderSheet();
    expect(screen.getByText("Anthropic")).toBeInTheDocument();
    expect(screen.getByText("OpenRouter")).toBeInTheDocument();
    expect(screen.queryByText("ChatGPT")).not.toBeInTheDocument();
  });

  it("connects with a key, then enables the recommended models it chose", async () => {
    const onConnected = jest.fn();
    renderSheet([], onConnected);

    fireEvent.click(screen.getByRole("button", { name: /Anthropic/ }));
    expect(screen.getByLabelText("Name")).toHaveValue("Anthropic");
    fireEvent.change(screen.getByLabelText(/API key/), { target: { value: "sk-ant-good" } });
    expect(await screen.findByText(/Key valid/, {}, VERIFY_TIMEOUT)).toBeInTheDocument();

    mockModels = [
      model({
        id: "m-new",
        display_name: "Claude New",
        profile: { family: "claude", release_date: "2026-09-01" },
      } as Partial<ModelWithProvider>),
      model({
        id: "m-old",
        display_name: "Claude Old",
        profile: { family: "claude", release_date: "2025-01-01" },
      } as Partial<ModelWithProvider>),
    ];
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));

    await waitFor(() =>
      expect(mockCreateProvider).toHaveBeenCalledWith({
        name: "Anthropic",
        provider_type: "anthropic",
        base_url: undefined,
        credentials: { api_key: "sk-ant-good" },
      }),
    );
    // Only the newest release of the family starts ticked.
    fireEvent.click(await screen.findByRole("button", { name: "Enable 1 model" }));
    await waitFor(() =>
      expect(mockSetModelsEnabled).toHaveBeenCalledWith([{ id: "m-new", enabled: true }]),
    );
    expect(mockReview).toHaveBeenCalledWith(["provider-new"]);
    expect(onConnected).toHaveBeenCalledWith("Anthropic connected. 1 model enabled.");
  });

  it("allows another instance of a driver but not a duplicate name", async () => {
    renderSheet([provider({ name: "Anthropic" })]);

    expect(screen.getByText("1 connected")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Anthropic/ }));
    expect(screen.getByText("Already connected")).toBeInTheDocument();
    const name = screen.getByLabelText("Name");
    expect(name).toHaveValue("Anthropic 2");

    fireEvent.change(name, { target: { value: "anthropic" } });
    expect(screen.getByText(/Names must be unique/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/API key/), { target: { value: "sk-ant-two" } });
    expect(screen.getByRole("button", { name: "Connect" })).toBeDisabled();
  });

  it("creates an OAuth provider without a key before handing off to sign-in", async () => {
    renderSheet();

    fireEvent.click(screen.getByRole("button", { name: /OpenRouter/ }));
    fireEvent.click(screen.getByRole("button", { name: "Sign in with OpenRouter" }));

    // The OAuth state binds to a provider id, so the provider is created
    // credential-free before the browser leaves for the authorize endpoint.
    await waitFor(() =>
      expect(mockCreateProvider).toHaveBeenCalledWith({
        name: "OpenRouter",
        provider_type: "openrouter",
        base_url: undefined,
        credentials: undefined,
      }),
    );
  });

  it("signs in to a personal ChatGPT plan and opens its settings page", async () => {
    mockChatGptEnabled = true;
    const popup = { opener: {}, location: { assign: jest.fn() }, close: jest.fn() };
    const open = jest.spyOn(window, "open").mockReturnValue(popup as unknown as Window);
    mockStartChatGptLogin.mockResolvedValue({ authorize_url: "https://auth.openai.com/authorize" });
    renderSheet();

    fireEvent.click(screen.getByRole("button", { name: /ChatGPT/ }));
    fireEvent.click(screen.getByRole("button", { name: /Sign in with ChatGPT/ }));

    await waitFor(() => expect(mockStartChatGptLogin).toHaveBeenCalledWith("provider-new"));
    expect(mockRouterPush).toHaveBeenCalledWith("/models/providers/provider-new");
    expect(popup.location.assign).toHaveBeenCalledWith("https://auth.openai.com/authorize");
    expect(popup.opener).toBeNull();
    open.mockRestore();
  });
});
