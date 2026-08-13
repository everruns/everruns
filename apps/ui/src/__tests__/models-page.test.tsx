import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ModelsPage from "@/app/(main)/models/page";

const mockUseSearchParams = jest.fn();

jest.mock("next/navigation", () => ({
  usePathname: () => "/models",
  useSearchParams: () => mockUseSearchParams(),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

const mockProviders = [
  {
    id: "provider-1",
    name: "OpenAI Production",
    provider_type: "openai",
    status: "active",
    api_key_set: true,
    base_url: "https://api.openai.com/v1",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
  {
    id: "provider-2",
    name: "Anthropic Dev",
    provider_type: "anthropic",
    status: "active",
    api_key_set: true,
    base_url: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

const mockModels = [
  {
    id: "model-1",
    model_id: "gpt-5.2",
    display_name: "GPT-4",
    provider_id: "provider-1",
    provider_name: "OpenAI Production",
    provider_type: "openai",
    healthy: true,
    enabled: true,
    capabilities: ["chat", "function_calling"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

const mockUseProviders = jest.fn();
const mockUseModels = jest.fn();
const mockUseCreateModel = jest.fn();
const mockUseDeleteModel = jest.fn();

jest.mock("@/hooks/use-providers", () => ({
  useProviders: () => mockUseProviders(),
  useModels: () => mockUseModels(),
  useCreateModel: () => mockUseCreateModel(),
  useDeleteModel: () => mockUseDeleteModel(),
}));

jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({
    data: {
      id: "org_1",
      name: "Test Org",
      default_model_id: "model-1",
      default_harness_id: null,
      base_harness_id: null,
      created_at: "2024-01-01",
      updated_at: "2024-01-01",
    },
    isLoading: false,
    error: null,
  }),
  useUpdateOrganization: () => ({
    mutateAsync: jest.fn(),
    isPending: false,
  }),
}));

jest.mock("@/lib/api/providers", () => ({
  updateModel: jest.fn(),
}));

jest.mock("@/lib/query-keys", () => ({
  queryKeys: {
    models: {
      all: ["models"],
      list: () => ["models"],
      detail: (id: string) => ["models", id],
    },
    organizations: { all: ["organizations"], detail: (id: string) => ["organization", id] },
  },
}));

describe("ModelsPage", () => {
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

    mockUseProviders.mockReturnValue({
      data: mockProviders,
      isLoading: false,
      error: null,
    });
    mockUseModels.mockReturnValue({
      data: mockModels,
      isLoading: false,
      error: null,
    });
    mockUseCreateModel.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
    mockUseDeleteModel.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
    mockUseSearchParams.mockReturnValue(new URLSearchParams());
  });

  it("renders Models section header", () => {
    render(<ModelsPage />, { wrapper });

    expect(screen.getByRole("heading", { level: 1, name: "Models" })).toBeInTheDocument();
    expect(
      screen.getByText("Manage the models available from your configured providers."),
    ).toBeInTheDocument();
  });

  it("renders model rows with correct data", () => {
    render(<ModelsPage />, { wrapper });

    expect(screen.getAllByText("GPT-4").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText(/gpt-5\.2 - OpenAI Production/)).toBeInTheDocument();
  });

  it("groups models into Enabled and Available sections", () => {
    mockUseModels.mockReturnValue({
      data: [
        ...mockModels,
        {
          id: "model-2",
          model_id: "gpt-3.5-turbo",
          display_name: "GPT-3.5",
          provider_id: "provider-1",
          provider_name: "OpenAI Production",
          provider_type: "openai",
          healthy: true,
          enabled: false,
          capabilities: ["chat"],
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    expect(screen.getByRole("heading", { name: "Enabled models" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Available models" })).toBeInTheDocument();
  });

  it("filters models by provider query parameter", () => {
    mockUseSearchParams.mockReturnValue(new URLSearchParams("provider=provider-2"));
    mockUseModels.mockReturnValue({
      data: [
        ...mockModels,
        {
          id: "model-2",
          model_id: "claude-sonnet-4-5",
          display_name: "Claude Sonnet 4.5",
          provider_id: "provider-2",
          provider_name: "Anthropic Dev",
          provider_type: "anthropic",
          healthy: true,
          enabled: true,
          capabilities: ["chat"],
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    expect(screen.getByText("Manage the models available from Anthropic Dev.")).toBeInTheDocument();
    expect(screen.getByText("Claude Sonnet 4.5")).toBeInTheDocument();
    expect(screen.queryByText(/gpt-5\.2 - OpenAI Production/)).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Clear filter/i })).toHaveAttribute("href", "/models");
  });

  it("sorts models within a section by release date descending", () => {
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "older",
          model_id: "gpt-4",
          display_name: "GPT-4 Older",
          provider_id: "provider-1",
          provider_name: "OpenAI Production",
          provider_type: "openai",
          healthy: true,
          enabled: true,
          capabilities: [],
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
          profile: {
            name: "GPT-4 Older",
            family: "gpt-4",
            attachment: false,
            reasoning: false,
            temperature: true,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            release_date: "2024-01-01",
          },
        },
        {
          id: "newer",
          model_id: "gpt-4.1",
          display_name: "GPT-4.1 Newer",
          provider_id: "provider-1",
          provider_name: "OpenAI Production",
          provider_type: "openai",
          healthy: true,
          enabled: true,
          capabilities: [],
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
          profile: {
            name: "GPT-4.1 Newer",
            family: "gpt-4.1",
            attachment: false,
            reasoning: false,
            temperature: true,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            release_date: "2025-04-14",
          },
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    const newer = screen.getByText("GPT-4.1 Newer");
    const older = screen.getByText("GPT-4 Older");
    expect(newer.compareDocumentPosition(older) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("places models without a release_date below dated models in the same section", () => {
    const baseModel = {
      provider_id: "provider-1",
      provider_name: "OpenAI Production",
      provider_type: "openai" as const,
      healthy: true,
      enabled: true,
      capabilities: [],
      created_at: "2024-01-01T00:00:00Z",
      updated_at: "2024-01-01T00:00:00Z",
    };
    const baseProfile = {
      family: "gpt",
      attachment: false,
      reasoning: false,
      temperature: true,
      tool_call: true,
      structured_output: true,
      open_weights: false,
    };
    mockUseModels.mockReturnValue({
      data: [
        {
          ...baseModel,
          id: "undated",
          model_id: "gpt-undated",
          display_name: "GPT Undated",
          profile: { ...baseProfile, name: "GPT Undated" },
        },
        {
          ...baseModel,
          id: "dated",
          model_id: "gpt-dated",
          display_name: "GPT Dated",
          profile: { ...baseProfile, name: "GPT Dated", release_date: "2024-01-01" },
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    const dated = screen.getByText("GPT Dated");
    const undated = screen.getByText("GPT Undated");
    expect(dated.compareDocumentPosition(undated) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("breaks release_date ties using created_at descending", () => {
    const baseModel = {
      provider_id: "provider-1",
      provider_name: "OpenAI Production",
      provider_type: "openai" as const,
      healthy: true,
      enabled: true,
      capabilities: [],
      updated_at: "2024-01-01T00:00:00Z",
    };
    const baseProfile = {
      family: "gpt",
      attachment: false,
      reasoning: false,
      temperature: true,
      tool_call: true,
      structured_output: true,
      open_weights: false,
      release_date: "2025-04-14",
    };
    mockUseModels.mockReturnValue({
      data: [
        {
          ...baseModel,
          id: "added-first",
          model_id: "gpt-a",
          display_name: "GPT Added First",
          created_at: "2024-01-01T00:00:00Z",
          profile: { ...baseProfile, name: "GPT Added First" },
        },
        {
          ...baseModel,
          id: "added-later",
          model_id: "gpt-b",
          display_name: "GPT Added Later",
          created_at: "2024-06-01T00:00:00Z",
          profile: { ...baseProfile, name: "GPT Added Later" },
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    const later = screen.getByText("GPT Added Later");
    const first = screen.getByText("GPT Added First");
    expect(later.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("shows empty state when no models exist", () => {
    mockUseModels.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    expect(screen.getByText("No models configured")).toBeInTheDocument();
  });

  it("shows error message when models fail to load", () => {
    mockUseModels.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<ModelsPage />, { wrapper });

    expect(screen.getByText(/Failed to load models/)).toBeInTheDocument();
  });

  it("disables Add Model button when no providers exist", () => {
    mockUseProviders.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    expect(screen.getByRole("button", { name: /Add Model/i })).toBeDisabled();
  });

  it("shows Enabled badge and Disable button for enabled model", () => {
    render(<ModelsPage />, { wrapper });

    expect(screen.getByText("Enabled")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Disable/i })).toBeInTheDocument();
  });

  it("renders organization default model settings", () => {
    render(<ModelsPage />, { wrapper });

    expect(screen.getByText("Organization Settings")).toBeInTheDocument();
    expect(screen.getByText("Default Model")).toBeInTheDocument();
  });
});
