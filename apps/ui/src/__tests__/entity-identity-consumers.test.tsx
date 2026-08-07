import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentCard } from "@/components/agents/agent-card";
import { AgentIdentityCard } from "@/app/(main)/agent-identities/page";
import { InstalledPluginCard } from "@/app/(main)/plugins/page";
import type { Agent, AgentIdentity, InstalledPlugin } from "@/lib/api/types";

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ locale: "en" }),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org-1" }, isLoading: false }),
}));

jest.mock("@/components/chat/streamdown-message", () => ({
  InlineStreamdownMessage: ({ children }: { children: string }) => <>{children}</>,
}));

const agent: Agent = {
  id: "agent_019fda100f037c008024046d6b3d74c0",
  name: "deep-research",
  display_name: "Deep Research",
  description: "Research agent",
  system_prompt: "Research carefully",
  harness_id: "harness_generic",
  default_model_id: null,
  tags: [],
  capabilities: [],
  status: "active",
  created_at: "2026-08-07T00:00:00Z",
  updated_at: "2026-08-07T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

const identity: AgentIdentity = {
  id: "identity_019fda100f037c008024046d6b3d74c0",
  name: "Researcher",
  description: "Research identity",
  locale: "en-US",
  timezone: "America/Chicago",
  status: "active",
  created_at: "2026-08-07T00:00:00Z",
  updated_at: "2026-08-07T00:00:00Z",
};

const plugin: InstalledPlugin = {
  id: "plugin_019fda100f037c008024046d6b3d74c0",
  name: "resend",
  display_name: "Resend",
  description: "Send transactional email",
  version: "1.0.0",
  pinned_sha: "e807ff60",
  marketplace: "everruns",
  capability_ref: "plugin:plugin_019fda100f037c008024046d6b3d74c0",
  status: "active",
  warnings: [],
  update_available: false,
  created_at: "2026-08-07T00:00:00Z",
  updated_at: "2026-08-07T00:00:00Z",
};

describe("entity identity consumers", () => {
  it("places the agent ID action beside the readable agent name", () => {
    render(<AgentCard agent={agent} />);

    const identityLine = screen.getByText("Deep Research").closest('[data-slot="entity-identity"]');
    expect(identityLine).toContainElement(
      screen.getByRole("button", { name: `Copy ID: ${agent.id}` }),
    );
  });

  it("uses the shared identity line without a duplicate raw agent identity ID", () => {
    render(<AgentIdentityCard identity={identity} />);

    expect(screen.queryByText(identity.id)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: `Copy ID: ${identity.id}` })).toBeInTheDocument();
  });

  it("keeps the full namespace-prefixed plugin ID out of the card layout", () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <InstalledPluginCard plugin={plugin} onUninstall={jest.fn()} />
      </QueryClientProvider>,
    );

    expect(screen.queryByText(plugin.capability_ref)).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `Copy ID: ${plugin.capability_ref}` }),
    ).toBeInTheDocument();
  });
});
