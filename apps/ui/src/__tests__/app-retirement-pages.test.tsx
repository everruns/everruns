import { render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import AppsPage from "@/app/(main)/apps/page";
import { AppDetail } from "@/components/apps/app-detail";
import type { Agent, App, Harness } from "@/lib/api/types";

const mockUseApps = jest.fn();
const mockUseApp = jest.fn();
const mockUseAgents = jest.fn();
const mockUseHarnesses = jest.fn();

jest.mock("@/hooks/use-apps", () => ({
  useApps: () => mockUseApps(),
  useApp: () => mockUseApp(),
  useDeleteApp: () => ({ mutate: jest.fn(), isPending: false }),
  usePublishApp: () => ({ mutate: jest.fn(), isPending: false }),
  useUnpublishApp: () => ({ mutate: jest.fn(), isPending: false }),
  useUpdateApp: () => ({ mutate: jest.fn(), isPending: false }),
}));

jest.mock("@/hooks", () => ({
  useAgents: () => mockUseAgents(),
  usePageTitle: jest.fn(),
}));

jest.mock("@/hooks/use-harnesses", () => ({
  useHarnesses: () => mockUseHarnesses(),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => false }),
}));

jest.mock("@/components/agent-identity/agent-identity-select", () => ({
  AgentIdentitySelect: () => null,
}));

jest.mock("@/components/apps/live-activity-rail", () => ({
  LiveActivityRail: () => null,
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

const agent: Agent = {
  id: "agent_019fda100f037c008024046d6b3d74c0",
  name: "customer-support",
  display_name: "Customer Support",
  description: "Handles customer requests.",
  system_prompt: "Help the customer.",
  harness_id: "harness_019fda100f037c008024046d6b3d74c0",
  default_model_id: null,
  tags: [],
  capabilities: [],
  status: "active",
  created_at: "2026-08-07T00:00:00Z",
  updated_at: "2026-08-07T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

const harness: Harness = {
  id: "harness_019fda100f037c008024046d6b3d74c0",
  name: "support-harness",
  display_name: "Support Harness",
  description: "Runs customer support agents.",
  system_prompt: "Help the customer.",
  parent_harness_id: null,
  default_model_id: null,
  tags: [],
  capabilities: [],
  initial_files: [],
  is_built_in: false,
  status: "active",
  created_at: "2026-08-07T00:00:00Z",
  updated_at: "2026-08-07T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

const app: App = {
  id: "app_019fda100f037c008024046d6b3d74c0",
  name: "Customer Support App",
  description: "Exposes customer support through existing channels.",
  harness_id: harness.id,
  agent_id: agent.id,
  agent_version_policy: "default",
  agent_version_id: null,
  agent_identity_id: null,
  owner_principal_id: "principal_019fda100f037c008024046d6b3d74c0",
  channels: [],
  status: "published",
  published_at: "2026-08-07T00:00:00Z",
  created_at: "2026-08-07T00:00:00Z",
  updated_at: "2026-08-07T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

describe("App retirement page adapters", () => {
  beforeEach(() => {
    mockUseApps.mockReturnValue({ data: [app], isLoading: false, error: null });
    mockUseApp.mockReturnValue({ data: app, isLoading: false });
    mockUseAgents.mockReturnValue({ data: [agent], isLoading: false, error: null });
    mockUseHarnesses.mockReturnValue({ data: [harness], isLoading: false, error: null });
  });

  it("links list Apps to the display names of their bound agents", () => {
    render(<AppsPage />);
    const banner = screen
      .getByText("Apps are moving to agent-owned endpoints")
      .closest<HTMLElement>('[data-slot="notice"]');
    expect(banner).not.toBeNull();
    expect(within(banner!).getByRole("link", { name: agent.display_name! })).toHaveAttribute(
      "href",
      `/agents/${agent.id}`,
    );
  });

  it("links the detail App to the display name of its bound agent", () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <AppDetail appId={app.id} />
      </QueryClientProvider>,
    );

    const banner = screen
      .getByText("Apps are moving to agent-owned endpoints")
      .closest<HTMLElement>('[data-slot="notice"]');
    expect(banner).not.toBeNull();
    expect(within(banner!).getByRole("link", { name: agent.display_name! })).toHaveAttribute(
      "href",
      `/agents/${agent.id}`,
    );
  });
});
