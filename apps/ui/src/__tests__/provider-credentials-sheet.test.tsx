import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ProviderCredentialsSheet } from "@/components/providers/provider-credentials";
import type { Provider } from "@/lib/api/types";

const mockUpdate = jest.fn();
const mockCheckCredentials = jest.fn();
const mockStartOAuth = jest.fn();

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
  data: { policies: {}, drivers: [keyDriver("anthropic"), keyDriver("openrouter", true)] },
  isLoading: false,
  error: null,
};

jest.mock("@/hooks/use-providers", () => ({
  useUpdateProvider: () => ({ mutateAsync: mockUpdate, isPending: false }),
  useProvidersConfig: () => mockConfig,
}));

jest.mock("@/lib/api/providers", () => ({
  checkProviderCredentials: (...args: unknown[]) => mockCheckCredentials(...args),
  providerSupportsOAuth: (driver: string) => driver === "openrouter",
  startProviderOAuth: (...args: unknown[]) => mockStartOAuth(...args),
}));

const provider = (overrides: Partial<Provider>): Provider =>
  ({
    id: "provider-1",
    name: "OpenRouter",
    provider_type: "openrouter",
    status: "active",
    api_key_set: true,
    managed: false,
    created_at: "2026-11-01T00:00:00Z",
    updated_at: "2026-11-15T20:12:00Z",
    ...overrides,
  }) as Provider;

const VERIFY_TIMEOUT = { timeout: 3000 };

describe("ProviderCredentialsSheet", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockCheckCredentials.mockResolvedValue({ status: "valid", models: 3 });
    mockUpdate.mockResolvedValue({});
    mockStartOAuth.mockResolvedValue(undefined);
  });

  const renderSheet = (p: Provider, onSaved = jest.fn(), onOpenChange = jest.fn()) =>
    render(
      <ProviderCredentialsSheet provider={p} open onOpenChange={onOpenChange} onSaved={onSaved} />,
    );

  it("shows the current credential and replaces it by signing in again", () => {
    renderSheet(provider({}));

    expect(screen.getByText("OpenRouter credentials")).toBeInTheDocument();
    expect(screen.getByText("Connected")).toBeInTheDocument();
    expect(screen.getByText("Set, encrypted at rest")).toBeInTheDocument();
    expect(screen.getByText("Replace credential")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Sign in again with OpenRouter/ })).toBeChecked();
    expect(screen.queryByLabelText(/^API key/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Continue to OpenRouter/ }));
    expect(mockStartOAuth).toHaveBeenCalledWith("provider-1");
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it("saves a pasted key once the live check passes", async () => {
    const onSaved = jest.fn();
    const onOpenChange = jest.fn();
    renderSheet(provider({}), onSaved, onOpenChange);

    fireEvent.click(screen.getByRole("radio", { name: /Paste a different API key/ }));
    const save = screen.getByRole("button", { name: "Save key" });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByLabelText(/^API key/), { target: { value: "sk-or-new" } });
    expect(await screen.findByText(/Key valid/, {}, VERIFY_TIMEOUT)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save key" }));

    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith({
        provider_type: "openrouter",
        credentials: { api_key: "sk-or-new" },
      }),
    );
    expect(onSaved).toHaveBeenCalledWith("OpenRouter key saved.");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(mockStartOAuth).not.toHaveBeenCalled();
  });

  it("keeps a rejected key from replacing a working one", async () => {
    mockCheckCredentials.mockResolvedValue({ status: "rejected", message: "Invalid key" });
    renderSheet(provider({ name: "Anthropic", provider_type: "anthropic" }));

    // No sign-in for this driver, so there is no choice to make.
    expect(screen.queryByText("Replace credential")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/^API key/), { target: { value: "sk-ant-bad" } });
    expect(await screen.findByText(/Key rejected/, {}, VERIFY_TIMEOUT)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save key" })).toBeDisabled();
  });
});
