import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ProviderDetailPage from "@/app/(main)/settings/providers/[providerId]/page";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    href,
    className,
  }: {
    children: React.ReactNode;
    href: string;
    className?: string;
  }) => (
    <a href={href} className={className}>
      {children}
    </a>
  ),
}));

const mockProvider = {
  id: "provider-1",
  name: "OpenAI Production",
  provider_type: "openai",
  status: "active",
  api_key_set: true,
  base_url: "https://api.openai.com/v1",
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
};

const mockModels = [
  {
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
  },
  {
    id: "model-2",
    model_id: "gpt-4.1",
    display_name: "GPT-4.1",
    provider_id: "provider-1",
    provider_name: "OpenAI Production",
    provider_type: "openai",
    healthy: true,
    enabled: false,
    capabilities: ["chat"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

const mockUseLlmProvider = jest.fn();
const mockUseLlmModels = jest.fn();
const mockUseUpdateLlmProvider = jest.fn();

jest.mock("@/hooks/use-llm-providers", () => ({
  useLlmProvider: () => mockUseLlmProvider(),
  useLlmModels: () => mockUseLlmModels(),
  useUpdateLlmProvider: () => mockUseUpdateLlmProvider(),
}));

describe("ProviderDetailPage", () => {
  let queryClient: QueryClient;
  let updateProvider: jest.Mock;

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
    updateProvider = jest.fn().mockResolvedValue(mockProvider);
    mockUseLlmProvider.mockReturnValue({ data: mockProvider, isLoading: false, error: null });
    mockUseLlmModels.mockReturnValue({ data: mockModels, isLoading: false, error: null });
    mockUseUpdateLlmProvider.mockReturnValue({ mutateAsync: updateProvider, isPending: false });
  });

  it("renders provider details with model counts", async () => {
    await act(async () => {
      render(<ProviderDetailPage params={Promise.resolve({ providerId: "provider-1" })} />, {
        wrapper,
      });
    });

    expect(
      await screen.findByRole("heading", { name: /OpenAI Production active/i }),
    ).toBeInTheDocument();
    expect(screen.getByDisplayValue("OpenAI Production")).toBeInTheDocument();
    expect(screen.getByText("2 models available")).toBeInTheDocument();
    expect(screen.getByText("1 enabled")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /View Models/i })).toHaveAttribute(
      "href",
      "/models?provider=provider-1",
    );
  });

  it("saves provider name changes", async () => {
    await act(async () => {
      render(<ProviderDetailPage params={Promise.resolve({ providerId: "provider-1" })} />, {
        wrapper,
      });
    });

    fireEvent.change(await screen.findByLabelText("Name"), {
      target: { value: "OpenAI Renamed" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Save Changes/i }));

    await waitFor(() => {
      expect(updateProvider).toHaveBeenCalledWith({ name: "OpenAI Renamed" });
    });
  });
});
