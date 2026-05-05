import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ModelsPage from "@/app/(main)/models/page";

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
];

const mockModels = [
  {
    id: "model-1",
    model_id: "gpt-5.2",
    display_name: "GPT-4",
    provider_id: "provider-1",
    provider_name: "OpenAI Production",
    provider_type: "openai",
    status: "active",
    enabled: true,
    capabilities: ["chat", "function_calling"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

const mockUseLlmProviders = jest.fn();
const mockUseLlmModels = jest.fn();
const mockUseCreateLlmModel = jest.fn();
const mockUseDeleteLlmModel = jest.fn();

jest.mock("@/hooks/use-llm-providers", () => ({
  useLlmProviders: () => mockUseLlmProviders(),
  useLlmModels: () => mockUseLlmModels(),
  useCreateLlmModel: () => mockUseCreateLlmModel(),
  useDeleteLlmModel: () => mockUseDeleteLlmModel(),
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

jest.mock("@/lib/api/llm-providers", () => ({
  updateLlmModel: jest.fn(),
}));

jest.mock("@/lib/query-keys", () => ({
  queryKeys: {
    llmModels: {
      all: ["llm-models"],
      list: () => ["llm-models"],
      detail: (id: string) => ["llm-models", id],
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

    mockUseLlmProviders.mockReturnValue({
      data: mockProviders,
      isLoading: false,
      error: null,
    });
    mockUseLlmModels.mockReturnValue({
      data: mockModels,
      isLoading: false,
      error: null,
    });
    mockUseCreateLlmModel.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
    mockUseDeleteLlmModel.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("renders Models section header", () => {
    render(<ModelsPage />, { wrapper });

    expect(screen.getByText("Models")).toBeInTheDocument();
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
    mockUseLlmModels.mockReturnValue({
      data: [
        ...mockModels,
        {
          id: "model-2",
          model_id: "gpt-3.5-turbo",
          display_name: "GPT-3.5",
          provider_id: "provider-1",
          provider_name: "OpenAI Production",
          provider_type: "openai",
          status: "active",
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

  it("sorts models within a section by release date descending", () => {
    mockUseLlmModels.mockReturnValue({
      data: [
        {
          id: "older",
          model_id: "gpt-4",
          display_name: "GPT-4 Older",
          provider_id: "provider-1",
          provider_name: "OpenAI Production",
          provider_type: "openai",
          status: "active",
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
          status: "active",
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

  it("shows empty state when no models exist", () => {
    mockUseLlmModels.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<ModelsPage />, { wrapper });

    expect(screen.getByText("No models configured")).toBeInTheDocument();
  });

  it("shows error message when models fail to load", () => {
    mockUseLlmModels.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<ModelsPage />, { wrapper });

    expect(screen.getByText(/Failed to load models/)).toBeInTheDocument();
  });

  it("disables Add Model button when no providers exist", () => {
    mockUseLlmProviders.mockReturnValue({
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
