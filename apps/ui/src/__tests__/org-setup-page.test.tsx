import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import OrgSetupPage from "@/app/(main)/orgs/[orgId]/setup/page";

const mockPush = jest.fn();
const mockSetCurrentOrg = jest.fn();
const mockCreateProvider = jest.fn();

jest.mock("next/navigation", () => ({
  useParams: () => ({ orgId: "org_test123" }),
  useRouter: () => ({ push: mockPush }),
}));

jest.mock("@/lib/api/organizations", () => ({
  getOrganization: jest.fn().mockResolvedValue({
    id: "org_test123",
    name: "Test Org",
    default_harness_id: "harness_default",
    base_harness_id: "harness_base",
  }),
  completeOrgOnboarding: jest.fn().mockResolvedValue({
    id: "org_test123",
    name: "Test Org",
    onboarding_completed_at: "2026-01-01T00:00:00Z",
  }),
}));

jest.mock("@/lib/api/harnesses", () => ({
  listHarnesses: jest.fn().mockResolvedValue([
    { id: "harness_default", name: "generic" },
    { id: "harness_base", name: "base" },
  ]),
}));

jest.mock("@/hooks/use-providers", () => ({
  useProviders: () => ({
    data: [],
    isLoading: false,
    isError: false,
  }),
  useCreateProvider: () => ({
    mutateAsync: mockCreateProvider,
    isPending: false,
  }),
}));

// The Done step links to the precreated Platform Chat thread; its own suite
// covers how the thread is ensured.
jest.mock("@/hooks/use-platform-chat-thread", () => ({
  usePlatformChatThread: () => ({ thread: { id: "ses_platform_chat" }, isLoading: false }),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({
    currentOrg: { public_id: "org_test123", name: "Test Org", role: "owner" },
    organizations: [{ public_id: "org_test123", name: "Test Org", role: "owner" }],
    setCurrentOrg: mockSetCurrentOrg,
    isSwitching: false,
  }),
}));

describe("OrgSetupPage", () => {
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
    mockPush.mockClear();
    mockSetCurrentOrg.mockClear();
    mockCreateProvider.mockClear();
  });

  it("offers OpenAI and Anthropic during org setup, but not Azure OpenAI", async () => {
    render(<OrgSetupPage />, { wrapper });

    expect(await screen.findByText("Setting up Test Org")).toBeInTheDocument();

    await waitFor(() => {
      // Card accessible names include the models subline, so match by prefix.
      expect(screen.getByRole("button", { name: /^OpenAI/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /^Anthropic/ })).toBeInTheDocument();
    });

    // The breadth strip mentions Azure OpenAI as plain text, but it must not
    // be a selectable card during setup.
    expect(screen.queryByRole("button", { name: /Azure OpenAI/ })).not.toBeInTheDocument();
  });

  it("ends onboarding in the precreated Platform Chat thread", async () => {
    render(<OrgSetupPage />, { wrapper });

    // Skipping the provider form is the shortest route to the Done step.
    const skip = await screen.findByRole("button", { name: "Skip for now" });
    fireEvent.click(skip);

    const open = await screen.findByRole("link", { name: /Open Platform Chat/ });
    expect(open).toHaveAttribute("href", "/chats/ses_platform_chat");
  });
});
