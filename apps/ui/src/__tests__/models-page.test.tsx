import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ModelsPage from "@/app/(main)/models/page";
import { updateModel } from "@/lib/api/providers";
import { ApiError } from "@/lib/api/client";
import type { ModelWithProvider } from "@/lib/api/types";

const mockUseSearchParams = jest.fn();
const mockRouterReplace = jest.fn();

jest.mock("next/navigation", () => ({
  usePathname: () => "/models",
  useSearchParams: () => mockUseSearchParams(),
  useRouter: () => ({ push: jest.fn(), replace: mockRouterReplace }),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

let mockCanManage = true;
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => mockCanManage }),
}));

jest.mock("@/hooks/use-chatgpt-connection", () => ({
  useChatGptConnection: () => ({ data: null }),
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
    capabilities: ["chat", "function_calling"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) as ModelWithProvider;

const mockUseProviders = jest.fn();
const mockUseModels = jest.fn();
const mockSetModelsEnabled = jest.fn();
const mockReview = jest.fn();
const mockUpdateOrganization = jest.fn((_data: unknown) => Promise.resolve());

jest.mock("@/hooks/use-providers", () => ({
  useDecisionDefault: () => ({
    data: null,
    setDefault: { isPending: false, mutateAsync: jest.fn() },
  }),
  useProvidersConfig: () => ({ data: undefined }),
  useModelProfiles: () => ({ data: [] }),
  useProviders: () => mockUseProviders(),
  useModels: () => mockUseModels(),
  useCreateModel: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useCreateProvider: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useDeleteModel: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useSyncProviderModels: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useUpdateProvider: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useSetModelsEnabled: () => ({ mutateAsync: mockSetModelsEnabled, isPending: false }),
  useReviewProviderModels: () => ({ mutateAsync: mockReview, isPending: false }),
}));

jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({
    data: {
      id: "org_1",
      name: "Test Org",
      default_model_id: "model-1",
      default_provider_per_service: {},
      system_decisions: "deployment",
      created_at: "2024-01-01",
      updated_at: "2024-01-01",
    },
    isLoading: false,
    error: null,
  }),
  useUpdateOrganization: () => ({
    mutateAsync: (data: unknown) => mockUpdateOrganization(data),
    isPending: false,
  }),
}));

jest.mock("@/lib/api/providers", () => ({
  updateModel: jest.fn(),
  checkProviderCredentials: jest.fn(),
  providerSupportsOAuth: jest.fn(() => false),
  providerOAuthAuthorizeUrl: jest.fn(),
}));

describe("ModelsPage", () => {
  let queryClient: QueryClient;
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  const setModels = (data: ModelWithProvider[]) =>
    mockUseModels.mockReturnValue({ data, isLoading: false, error: null });

  beforeEach(() => {
    jest.clearAllMocks();
    mockCanManage = true;
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    mockUseProviders.mockReturnValue({ data: mockProviders, isLoading: false, error: null });
    setModels([model({})]);
    mockUseSearchParams.mockReturnValue(new URLSearchParams());
    mockSetModelsEnabled.mockResolvedValue({ failed: [] });
    mockReview.mockResolvedValue(undefined);
  });

  it("renders the Models, Providers and Defaults tabs", () => {
    render(<ModelsPage />, { wrapper });
    expect(screen.getByRole("heading", { level: 1, name: "Models" })).toBeInTheDocument();
    for (const name of [/^Models/, /^Providers/, /^Defaults/]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
    expect(screen.getByRole("button", { name: "Connect provider" })).toBeInTheDocument();
  });

  it("switches tabs through the URL, keeping other parameters", () => {
    mockUseSearchParams.mockReturnValue(new URLSearchParams("provider=provider-1"));
    render(<ModelsPage />, { wrapper });
    fireEvent.click(screen.getByRole("tab", { name: /^Providers/ }));
    expect(mockRouterReplace).toHaveBeenCalledWith("/models?provider=provider-1&tab=providers", {
      scroll: false,
    });
  });

  it("labels rows and marks the org default", () => {
    render(<ModelsPage />, { wrapper });
    expect(screen.getByText("GPT-5.2")).toBeInTheDocument();
    expect(screen.getByText("Org default")).toBeInTheDocument();
  });

  it("shows enabled models by default once there are any", () => {
    setModels([model({}), model({ id: "model-2", display_name: "GPT Old", enabled: false })]);
    render(<ModelsPage />, { wrapper });
    expect(screen.getByText("GPT-5.2")).toBeInTheDocument();
    expect(screen.queryByText("GPT Old")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Enabled/, pressed: true })).toBeInTheDocument();
  });

  it("filters by the provider in the URL", () => {
    mockUseSearchParams.mockReturnValue(new URLSearchParams("provider=provider-2&status=all"));
    setModels([
      model({}),
      model({
        id: "model-2",
        display_name: "Claude Sonnet 5.5",
        provider_id: "provider-2",
        provider_name: "Anthropic Dev",
        provider_type: "anthropic",
      }),
    ]);
    render(<ModelsPage />, { wrapper });
    expect(screen.getByText("Claude Sonnet 5.5")).toBeInTheDocument();
    expect(screen.queryByText("GPT-5.2")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Clear provider filter/ })).toHaveAttribute(
      "href",
      "/models",
    );
  });

  it("sorts newest release first, undated last", () => {
    mockUseSearchParams.mockReturnValue(new URLSearchParams("status=all"));
    setModels([
      model({ id: "undated", display_name: "GPT Undated" }),
      model({
        id: "older",
        display_name: "GPT Older",
        profile: { release_date: "2024-01-01" },
      } as Partial<ModelWithProvider>),
      model({
        id: "newer",
        display_name: "GPT Newer",
        profile: { release_date: "2025-04-14" },
      } as Partial<ModelWithProvider>),
    ]);
    render(<ModelsPage />, { wrapper });
    const order = ["GPT Newer", "GPT Older", "GPT Undated"].map((name) => screen.getByText(name));
    expect(
      order[0].compareDocumentPosition(order[1]) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      order[1].compareDocumentPosition(order[2]) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("pages a large catalog", () => {
    mockUseSearchParams.mockReturnValue(new URLSearchParams("status=all"));
    setModels(
      Array.from({ length: 120 }, (_, i) =>
        model({ id: `m-${i}`, display_name: `Model ${i}`, enabled: false }),
      ),
    );
    render(<ModelsPage />, { wrapper });
    expect(screen.getByText("Showing 50 of 120")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show 50 more" }));
    expect(screen.getByText("Showing 100 of 120")).toBeInTheDocument();
  });

  it("reviews new models in bulk and marks the provider reviewed", async () => {
    setModels([
      model({}),
      model({ id: "new-1", display_name: "GPT New", enabled: false, is_new: true }),
    ]);
    render(<ModelsPage />, { wrapper });
    expect(screen.getByText(/1 new model discovered/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Review" }));

    const drawer = await screen.findByRole("dialog");
    fireEvent.click(within(drawer).getByRole("checkbox", { name: /GPT New/ }));
    fireEvent.click(within(drawer).getByRole("button", { name: "Enable 1 model" }));

    await waitFor(() =>
      expect(mockSetModelsEnabled).toHaveBeenCalledWith([{ id: "new-1", enabled: true }]),
    );
    expect(mockReview).toHaveBeenCalledWith(["provider-1"]);
  });

  it("is read-only without the models permission", () => {
    mockCanManage = false;
    render(<ModelsPage />, { wrapper });
    expect(screen.queryByRole("button", { name: "Connect provider" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Disable/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Add model/ })).not.toBeInTheDocument();
  });

  it("shows an error when models fail to load", () => {
    mockUseModels.mockReturnValue({ data: [], isLoading: false, error: new Error("Network") });
    render(<ModelsPage />, { wrapper });
    expect(screen.getByText(/Failed to load models/)).toBeInTheDocument();
  });

  it("disables Add model when no providers exist", () => {
    mockUseProviders.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<ModelsPage />, { wrapper });
    expect(screen.getByRole("button", { name: /Add model/ })).toBeDisabled();
  });

  it("lets the organization answer system decisions with its own model", async () => {
    mockUseSearchParams.mockReturnValue(new URLSearchParams("tab=defaults"));
    render(<ModelsPage />, { wrapper });
    const trigger = screen.getByRole("combobox", { name: "System decisions" });
    expect(trigger).toHaveTextContent("Deployment default");
    fireEvent.click(trigger);
    const option = await screen.findByRole("option", {
      name: "This organization's decision model",
    });
    fireEvent.pointerDown(option, { pointerType: "mouse" });
    fireEvent.click(option);
    await waitFor(() =>
      expect(mockUpdateOrganization).toHaveBeenCalledWith({ system_decisions: "organization" }),
    );
  });

  // EVE-954 / Sentry EVERRUNS-1Y: a rejected action used to escape the async
  // handler unhandled — nothing on screen, the control silently reverting, and
  // the rejection reported to Sentry as an unhandled promise rejection.
  it("reports a failed toggle to the operator instead of rejecting unhandled", async () => {
    (updateModel as jest.Mock).mockRejectedValueOnce(
      new ApiError(404, "Not Found", "Model not found"),
    );
    const unhandled = jest.fn();
    window.addEventListener("unhandledrejection", unhandled);

    render(<ModelsPage />, { wrapper });
    fireEvent.click(screen.getByRole("button", { name: /Disable/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Failed to disable model: Model not found",
    );
    expect(unhandled).not.toHaveBeenCalled();
    window.removeEventListener("unhandledrejection", unhandled);
  });

  it("clears a previous failure when the next action succeeds", async () => {
    (updateModel as jest.Mock)
      .mockRejectedValueOnce(new ApiError(404, "Not Found", "Model not found"))
      .mockResolvedValueOnce(undefined);

    render(<ModelsPage />, { wrapper });
    fireEvent.click(screen.getByRole("button", { name: /Disable/i }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Disable/i }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  });

  it("reports a non-Error rejection without rendering [object Object]", async () => {
    (updateModel as jest.Mock).mockRejectedValueOnce({ nope: true });
    render(<ModelsPage />, { wrapper });
    fireEvent.click(screen.getByRole("button", { name: /Disable/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Failed to disable model: Unexpected error",
    );
  });
});
