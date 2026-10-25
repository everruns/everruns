import { PageChatProvider } from "@/test-utils/page-chat-provider";
import { type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import ProviderDetailPage from "@/app/(main)/models/providers/[providerId]/page";

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

jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: jest.fn(), push: jest.fn() }),
  usePathname: () => "/models/providers/provider-1",
  useSearchParams: () => new URLSearchParams(),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => true }),
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

const mockUseProvider = jest.fn();
const mockUseModels = jest.fn();
const mockUseUpdateProvider = jest.fn();

jest.mock("@/hooks/use-providers", () => ({
  useProvider: () => mockUseProvider(),
  useModels: () => mockUseModels(),
  useUpdateProvider: () => mockUseUpdateProvider(),
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
    mockUseProvider.mockReturnValue({ data: mockProvider, isLoading: false, error: null });
    mockUseModels.mockReturnValue({ data: mockModels, isLoading: false, error: null });
    mockUseUpdateProvider.mockReturnValue({ mutateAsync: updateProvider, isPending: false });
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

  it("does not show a ChatGPT Plan provider through a saved detail URL", async () => {
    mockUseProvider.mockReturnValue({
      data: { ...mockProvider, provider_type: "chatgpt" },
      isLoading: false,
      error: null,
    });
    await act(async () => {
      render(<ProviderDetailPage params={Promise.resolve({ providerId: "provider-1" })} />, {
        wrapper,
      });
    });
    expect(screen.getByText("Provider not found")).toBeInTheDocument();
    expect(screen.queryByDisplayValue("OpenAI Production")).not.toBeInTheDocument();
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

  it("does not reset an in-progress name edit after provider refetch", async () => {
    let provider = mockProvider;
    mockUseProvider.mockImplementation(() => ({
      data: provider,
      isLoading: false,
      error: null,
    }));

    let rerender: ReturnType<typeof render>["rerender"];
    await act(async () => {
      ({ rerender } = render(
        <ProviderDetailPage params={Promise.resolve({ providerId: "provider-1" })} />,
        { wrapper },
      ));
    });

    fireEvent.change(await screen.findByLabelText("Name"), {
      target: { value: "Draft Name" },
    });

    provider = { ...mockProvider };
    await act(async () => {
      rerender(<ProviderDetailPage params={Promise.resolve({ providerId: "provider-1" })} />);
    });

    expect(screen.getByDisplayValue("Draft Name")).toBeInTheDocument();
  });
  it("stages only the requested independent form and never reads or changes credential inputs", async () => {
    global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
    const adapters = new Map<string, PageDraftAdapter>();
    const mode: ChatModeState = {
      page: { route: "/models/providers/provider-1", label: "Provider" },
      pageContainer: { current: null },
      open: true,
      setOpen: () => {},
      actions: [],
      recordAction: () => {},
      registerPage: (page, adapter) => {
        if (adapter) adapters.set(page.form_name!, adapter);
        return () => {};
      },
    };
    await act(async () => {
      render(
        <PageChatProvider value={mode}>
          <ProviderDetailPage params={Promise.resolve({ providerId: "provider-1" })} />
        </PageChatProvider>,
        { wrapper },
      );
    });
    expect([...adapters.keys()].sort()).toEqual(["advanced", "details", "tracing"]);
    fireEvent.click(screen.getByRole("button", { name: "Add header" }));
    fireEvent.change(screen.getByLabelText("Header 1 name"), { target: { value: "x-fixture" } });
    fireEvent.change(screen.getByLabelText("Header 1 value"), {
      target: { value: "fixture-secret" },
    });
    const adapter = adapters.get("advanced")!;
    const before = adapter.read(adapter.binding!);
    expect(JSON.stringify(before)).not.toContain("fixture-secret");
    expect(() => adapter.patch(before.binding, before.revision, { headers: [] })).toThrow(
      /unsupported/,
    );
    act(() => {
      adapter.patch(before.binding, before.revision, { cache_diagnostics: true });
    });
    expect(screen.getByLabelText("Prompt cache diagnostics")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByLabelText("Header 1 value")).toHaveValue("fixture-secret");
    expect(screen.getByLabelText("Name")).toHaveValue(mockProvider.name);
    expect(updateProvider).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save Advanced Settings" }));
    await waitFor(() =>
      expect(updateProvider).toHaveBeenCalledWith({
        request_options: {
          cache_diagnostics: true,
          headers: [{ name: "x-fixture", value: "fixture-secret" }],
        },
      }),
    );
  });
});
