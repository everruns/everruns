import { render, screen, fireEvent, within } from "@testing-library/react";
import { ProvidersTab } from "@/components/providers/providers-tab";
import {
  DefaultsTab,
  modelDefaultProblem,
  providerDefaultProblem,
} from "@/components/models/defaults-tab";
import { CredentialFields } from "@/components/providers/credential-fields";
import ProvidersRedirect from "@/app/(main)/settings/providers/page";
import type { ModelWithProvider, Provider } from "@/lib/api/types";

const mockRedirect = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn() }),
  redirect: (url: string) => mockRedirect(url),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    href,
    prefetch: _prefetch,
    ...props
  }: React.AnchorHTMLAttributes<HTMLAnchorElement> & { href: string; prefetch?: boolean }) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

const mockSync = jest.fn();
const mockConfig = { data: { policies: {}, drivers: [] }, isLoading: false };
jest.mock("@/hooks/use-providers", () => ({
  useSyncProviderModels: () => ({ mutateAsync: mockSync, isPending: false }),
  useUpdateProvider: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useProvidersConfig: () => mockConfig,
  useDecisionDefault: () => ({
    data: null,
    setDefault: { mutateAsync: jest.fn(), isPending: false },
  }),
}));

jest.mock("@/hooks/use-chatgpt-connection", () => ({
  useChatGptConnection: () => mockConnection,
}));

const mockConnection = { data: { email: "me@example.com" } };
let mockOrg: Record<string, unknown> = {};
jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({ data: mockOrg, isLoading: false, error: null }),
  useUpdateOrganization: () => ({ mutateAsync: jest.fn(), isPending: false }),
}));

jest.mock("@/lib/api/providers", () => ({
  checkProviderCredentials: jest.fn(),
  providerSupportsOAuth: jest.fn(() => false),
  providerOAuthAuthorizeUrl: jest.fn((id: string) => `/api/v1/providers/${id}/oauth/authorize`),
}));

const provider = (overrides: Partial<Provider>): Provider =>
  ({
    id: "provider-1",
    name: "OpenAI Production",
    provider_type: "openai",
    status: "active",
    api_key_set: true,
    managed: false,
    base_url: "https://api.openai.com/v1",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) as Provider;

const model = (overrides: Partial<ModelWithProvider>): ModelWithProvider =>
  ({
    id: "model-1",
    model_id: "gpt-5.2",
    display_name: "GPT-5.2",
    provider_id: "provider-1",
    provider_name: "OpenAI Production",
    provider_type: "openai",
    healthy: true,
    enabled: true,
    capabilities: ["chat"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) as ModelWithProvider;

const providers = [
  provider({}),
  provider({
    id: "provider-2",
    name: "Anthropic Dev",
    provider_type: "anthropic",
    api_key_set: false,
    base_url: undefined,
  }),
];

describe("ProvidersTab", () => {
  const renderTab = (props: Partial<React.ComponentProps<typeof ProvidersTab>> = {}) =>
    render(
      <ProvidersTab
        providers={providers}
        providersLoading={false}
        providersError={null}
        models={[
          model({}),
          model({ id: "model-2", enabled: false, is_new: true }),
          model({ id: "model-3", stale: true }),
        ]}
        modelsLoading={false}
        canManage
        onConnect={jest.fn()}
        onReviewNew={jest.fn()}
        onNotice={jest.fn()}
        {...props}
      />,
    );

  it("summarizes each provider and links to its models and settings", () => {
    renderTab();
    // The stale model is no longer listed, so it does not count.
    expect(screen.getByRole("link", { name: "1 of 2 models enabled →" })).toHaveAttribute(
      "href",
      "/models?provider=provider-1",
    );
    expect(screen.getByRole("link", { name: "Settings for OpenAI Production" })).toHaveAttribute(
      "href",
      "/models/providers/provider-1",
    );
    expect(screen.getByText("Configured")).toBeInTheDocument();
    expect(screen.getByText("Not set")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Set key/ })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Update key for OpenAI Production" }),
    ).toBeInTheDocument();
  });

  it("offers a review of models a sync discovered", () => {
    const onReviewNew = jest.fn();
    renderTab({ onReviewNew });
    fireEvent.click(screen.getByRole("button", { name: /1 new model/ }));
    expect(onReviewNew).toHaveBeenCalledWith(expect.objectContaining({ id: "provider-1" }));
  });

  it("keeps host-managed credentials read-only", () => {
    renderTab({ providers: [provider({ managed: true })] });
    expect(screen.getByText("Host managed")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /key/i })).not.toBeInTheDocument();
  });

  it("is read-only for members without the models permission", () => {
    renderTab({ canManage: false });
    expect(screen.getByText(/Only organization admins connect/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Set key|Update key|Sync models/ }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /new model/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Connect provider" })).not.toBeInTheDocument();
  });

  it("labels a personal ChatGPT provider with its owner's sign-in", () => {
    renderTab({
      providers: [provider({ id: "p-chatgpt", name: "ChatGPT", provider_type: "chatgpt" })],
    });
    expect(screen.getByText("Personal")).toBeInTheDocument();
    expect(screen.getByText("You · me@example.com")).toBeInTheDocument();
  });
});

describe("DefaultsTab", () => {
  beforeEach(() => {
    window.HTMLElement.prototype.scrollIntoView = jest.fn();
    mockOrg = { id: "org_1", default_model_id: "model-1", default_provider_per_service: {} };
  });

  it("excludes personal providers from shared provider defaults", () => {
    render(
      <DefaultsTab
        models={[model({})]}
        providers={[
          ...providers,
          provider({ id: "personal", name: "Personal ChatGPT", provider_type: "chatgpt" }),
        ]}
        canManage
      />,
    );
    const embeddings = screen.getByLabelText("Embeddings");
    fireEvent.click(within(embeddings).getByRole("combobox"));
    expect(screen.getByRole("option", { name: "OpenAI Production" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Personal ChatGPT" })).not.toBeInTheDocument();
  });

  it("says there is no fallback when system decisions use the org's model", () => {
    mockOrg = { ...mockOrg, system_decisions: "organization" };
    render(<DefaultsTab models={[model({})]} providers={providers} canManage />);
    expect(screen.getByText(/no fallback to deployment keys/)).toBeInTheDocument();
    expect(screen.getByText(/No decision model is set/)).toBeInTheDocument();
  });

  it("explains why a default cannot be used", () => {
    expect(modelDefaultProblem("model-1", [model({})])).toBeNull();
    expect(modelDefaultProblem("gone", [])).toMatch(/no longer exists/);
    expect(modelDefaultProblem("model-1", [model({ enabled: false })])).toMatch(/disabled/);
    expect(modelDefaultProblem("model-1", [model({ stale: true })])).toMatch(/no longer lists/);
    expect(providerDefaultProblem("provider-2", providers)).toMatch(/no key/);
    expect(providerDefaultProblem(undefined, providers)).toBeNull();
  });
});

describe("CredentialFields", () => {
  const apiKeySchema = {
    fields: [
      { name: "api_key", label: "API Key", field_type: "password" as const, required: true },
    ],
    instructions_markdown: "",
  };

  it("marks required ungrouped credentials as required by default", () => {
    render(
      <CredentialFields schema={apiKeySchema} values={{}} onChange={jest.fn()} idPrefix="c" />,
    );
    expect(screen.getByLabelText("API Key")).toBeRequired();
  });

  it("does not require credentials when the add flow allows empty OAuth setup", () => {
    render(
      <CredentialFields
        schema={apiKeySchema}
        values={{}}
        onChange={jest.fn()}
        idPrefix="c"
        allowEmptySubmit
      />,
    );
    expect(screen.getByLabelText("API Key")).not.toBeRequired();
  });
});

describe("/settings/providers", () => {
  it("redirects to the Providers tab", () => {
    ProvidersRedirect();
    expect(mockRedirect).toHaveBeenCalledWith("/models?tab=providers");
  });
});
