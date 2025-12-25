import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ProvidersPage from "@/app/(main)/settings/providers/page";

// Mock the LLM providers hooks
const mockProviders = [
  {
    id: "provider-1",
    name: "OpenAI Production",
    provider_type: "openai",
    status: "active",
    is_default: true,
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
    is_default: false,
    api_key_set: false,
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
    status: "active",
    is_default: true,
    capabilities: ["chat", "function_calling"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

const mockUseLlmProviders = jest.fn();
const mockUseLlmModels = jest.fn();
const mockUseCreateLlmProvider = jest.fn();
const mockUseUpdateLlmProvider = jest.fn();
const mockUseDeleteLlmProvider = jest.fn();
const mockUseCreateLlmModel = jest.fn();
const mockUseDeleteLlmModel = jest.fn();

jest.mock("@/hooks/use-llm-providers", () => ({
  useLlmProviders: () => mockUseLlmProviders(),
  useLlmModels: () => mockUseLlmModels(),
  useCreateLlmProvider: () => mockUseCreateLlmProvider(),
  useUpdateLlmProvider: () => mockUseUpdateLlmProvider(),
  useDeleteLlmProvider: () => mockUseDeleteLlmProvider(),
  useCreateLlmModel: () => mockUseCreateLlmModel(),
  useDeleteLlmModel: () => mockUseDeleteLlmModel(),
}));

describe("ProvidersPage", () => {
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

    // Default mock implementations
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

    mockUseDeleteLlmProvider.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseDeleteLlmModel.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseCreateLlmProvider.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseUpdateLlmProvider.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseCreateLlmModel.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("renders LLM Providers section header", () => {
    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText("LLM Providers")).toBeInTheDocument();
    expect(
      screen.getByText("Configure the LLM providers that your agents can use.")
    ).toBeInTheDocument();
  });

  it("renders Models section header", () => {
    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText("Models")).toBeInTheDocument();
    expect(
      screen.getByText("Manage the models available from your configured providers.")
    ).toBeInTheDocument();
  });

  it("renders provider cards with correct data", () => {
    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText("OpenAI Production")).toBeInTheDocument();
    expect(screen.getByText("Anthropic Dev")).toBeInTheDocument();
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Anthropic")).toBeInTheDocument();
  });

  it("renders model rows with correct data", () => {
    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText("GPT-4")).toBeInTheDocument();
    expect(screen.getByText("gpt-5.2 - OpenAI Production")).toBeInTheDocument();
  });

  it("shows loading skeleton when providers are loading", () => {
    mockUseLlmProviders.mockReturnValue({
      data: [],
      isLoading: true,
      error: null,
    });

    render(<ProvidersPage />, { wrapper });

    // Check for skeleton elements (they have class containing 'animate-pulse')
    const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
    expect(skeletons.length).toBeGreaterThan(0);
  });

  it("shows empty state when no providers exist", () => {
    mockUseLlmProviders.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText("No providers configured")).toBeInTheDocument();
    expect(
      screen.getByText("Add an LLM provider to start using AI models with your agents.")
    ).toBeInTheDocument();
  });

  it("shows empty state when no models exist", () => {
    mockUseLlmModels.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText("No models configured")).toBeInTheDocument();
  });

  it("shows error message when providers fail to load", () => {
    mockUseLlmProviders.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText(/Failed to load providers/)).toBeInTheDocument();
  });

  it("shows error message when models fail to load", () => {
    mockUseLlmModels.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<ProvidersPage />, { wrapper });

    expect(screen.getByText(/Failed to load models/)).toBeInTheDocument();
  });

  it("renders Add Provider button", () => {
    render(<ProvidersPage />, { wrapper });

    const addButtons = screen.getAllByRole("button", { name: /Add Provider/i });
    expect(addButtons.length).toBeGreaterThan(0);
  });

  it("renders Add Model button", () => {
    render(<ProvidersPage />, { wrapper });

    const addButtons = screen.getAllByRole("button", { name: /Add Model/i });
    expect(addButtons.length).toBeGreaterThan(0);
  });

  it("disables Add Model button when no providers exist", () => {
    mockUseLlmProviders.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<ProvidersPage />, { wrapper });

    // The Add Model button in the header should be disabled
    const addModelButtons = screen.getAllByRole("button", { name: /Add Model/i });
    const headerButton = addModelButtons[0];
    expect(headerButton).toBeDisabled();
  });

  it("shows default star icon for default provider", () => {
    render(<ProvidersPage />, { wrapper });

    // OpenAI Production is the default provider
    const providerCard = screen.getByText("OpenAI Production").closest("div");
    expect(providerCard).toBeInTheDocument();
    // Check for the star icon (it has fill-yellow-500 class)
    const starIcons = document.querySelectorAll('[class*="fill-yellow-500"]');
    expect(starIcons.length).toBeGreaterThan(0);
  });

  it("shows API Key status correctly", () => {
    render(<ProvidersPage />, { wrapper });

    // OpenAI has API key set
    expect(screen.getByText(/API Key: Configured/i)).toBeInTheDocument();
    // Anthropic does not have API key set
    expect(screen.getByText(/API Key: Not set/i)).toBeInTheDocument();
  });

  it("opens Add Provider dialog when clicking Add Provider button", async () => {
    render(<ProvidersPage />, { wrapper });

    const addButton = screen.getAllByRole("button", { name: /Add Provider/i })[0];
    fireEvent.click(addButton);

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
      expect(screen.getByText("Add LLM Provider")).toBeInTheDocument();
    });
  });
});
