import { fireEvent, render, screen, within } from "@testing-library/react";
import { WebSearchShell as WebSearchPage } from "@/app/(main)/web-search/(tabs)/web-search-shell";
import { BRAVE_ACCOUNT_ID } from "@/app/dev/_fixtures/web-search-fixtures";

let mockTab: string | null = null;
const mockRouterReplace = jest.fn();
let mockAdmin = true;
const mockSave = jest.fn((_defaults: unknown) => Promise.resolve());
const mockCatalog = {
  built_in: [
    { id: "free", name: "Free search", description: "Keyless", price_note: null },
    {
      id: "native",
      name: "Model provider search",
      description: "The model's own",
      price_note: "Billed by the model provider, about $10 per 1,000 searches",
    },
  ],
  native_providers: [
    {
      provider_id: "provider-1",
      name: "OpenAI Production",
      provider_type: "openai",
      price_note: "Billed by the model provider, about $10 per 1,000 searches",
    },
  ],
  connectors: [
    {
      provider: "brave_search",
      name: "Brave Search",
      description: "Independent index with freshness filters.",
      icon: "search",
    },
  ],
  accounts: [
    {
      id: "00000000-0000-0000-0000-000000000007",
      name: "Team",
      provider: "brave_search",
      status: "ready",
      automatic_position: 2,
      connected_at: "2026-10-11T00:00:00Z",
    },
  ],
  defaults: {
    provider: "auto",
    automatic_order: ["native", "free"],
    allow_native: true,
    fallback: "next",
    results_per_search: 8,
    blocked_domains: ["pinterest.com"],
  },
};

jest.mock("next/navigation", () => ({
  usePathname: () => (mockTab ? `/web-search/${mockTab}` : "/web-search"),
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: jest.fn(), replace: mockRouterReplace }),
}));

jest.mock("@/hooks", () => ({ usePageTitle: () => undefined }));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ hasRole: () => mockAdmin, currentOrg: { public_id: "org_1" } }),
}));

jest.mock("@/hooks/use-search-providers", () => ({
  useSearchProviders: () => ({ data: mockCatalog, isLoading: false, error: null }),
  useSaveWebSearchDefaults: () => ({ mutateAsync: mockSave, isPending: false }),
  useInvalidateSearchProviders: () => () => Promise.resolve(),
}));

jest.mock("@/hooks/use-providers", () => ({
  useProviders: () => ({
    data: [
      { id: "provider-1", name: "OpenAI Production", provider_type: "openai" },
      { id: "provider-2", name: "Mistral", provider_type: "mistral" },
    ],
  }),
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useConnectionProviders: () => ({
    data: [
      {
        provider_id: "brave_search",
        display_name: "Brave Search",
        description: "Brave Search API",
        icon: "search",
        connection_type: "api_key",
        capabilities: ["web_search"],
        form_schema: {
          fields: [{ name: "api_key", label: "API key", field_type: "password", required: true }],
          instructions_markdown: "",
        },
      },
    ],
  }),
}));

jest.mock("@/hooks/use-organization-connections", () => ({
  useDeleteOrganizationConnection: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useSaveOrganizationConnection: () => ({ mutateAsync: jest.fn(), isPending: false }),
}));

beforeEach(() => {
  mockTab = null;
  mockAdmin = true;
  mockSave.mockClear();
});

describe("Web search page", () => {
  it("shows a card per provider with cost, status and role", () => {
    render(<WebSearchPage />);
    const native = screen.getByTestId("native-search-card");
    expect(within(native).getByText("Allowed")).toBeInTheDocument();
    expect(
      within(native).getByText("Billed by the model provider, about $10 per 1,000 searches"),
    ).toBeInTheDocument();
    expect(within(native).getByText("Mistral")).toBeInTheDocument();
    expect(within(native).getByText("1 of your 2 model providers support it")).toBeInTheDocument();
    // OpenAI, Anthropic, Gemini and OpenRouter drivers run native search.
    expect(within(native).getByText("Anthropic · not connected")).toBeInTheDocument();
    expect(within(native).getByText("Gemini · not connected")).toBeInTheDocument();
    expect(within(native).queryByText("Coming next")).not.toBeInTheDocument();
    expect(within(native).queryByText(/do not run it yet/)).not.toBeInTheDocument();

    const account = screen.getByTestId("search-account-card");
    expect(within(account).getByText("Search account · Team")).toBeInTheDocument();
    expect(within(account).getByText("Active")).toBeInTheDocument();
    expect(within(account).getByText("Automatic #2")).toBeInTheDocument();

    expect(screen.getByTestId("free-search-card")).toHaveTextContent("Last resort in every plan");
    expect(screen.getAllByText("Connect search provider").length).toBeGreaterThan(0);
  });

  it("opens the connector form from the connect card", () => {
    render(<WebSearchPage />);
    fireEvent.click(screen.getByRole("button", { name: /Connect search provider Brave Search/ }));
    expect(screen.getByRole("heading", { name: "Connect Brave Search" })).toBeInTheDocument();
    expect(screen.getByLabelText("API key")).toBeInTheDocument();
  });

  it("hides admin actions from members", () => {
    mockAdmin = false;
    render(<WebSearchPage />);
    expect(screen.queryByText("Connect search provider")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Disconnect/ })).not.toBeInTheDocument();
  });

  it("edits the Automatic order with free search pinned last", () => {
    mockTab = "defaults";
    render(<WebSearchPage />);
    const order = screen.getByRole("list", { name: "Automatic order" });
    expect(
      within(order)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual([
      expect.stringContaining("Model provider search"),
      expect.stringContaining("Brave Search · Team"),
      expect.stringContaining("Free search"),
    ]);
    expect(within(order).getByText("≈ $10 / 1k")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Move Free search up" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Move Brave Search · Team down" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Move Brave Search · Team up" }));
    expect(mockSave).toHaveBeenCalledWith(
      expect.objectContaining({ automatic_order: [BRAVE_ACCOUNT_ID, "native", "free"] }),
    );
  });

  it("turns model provider search off and says what it costs", () => {
    mockTab = "defaults";
    render(<WebSearchPage />);
    expect(
      screen.getByText(/about \$10 per 1,000 searches, on that provider's account/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Allow model provider search" }));
    expect(mockSave).toHaveBeenCalledWith(expect.objectContaining({ allow_native: false }));
  });
});
