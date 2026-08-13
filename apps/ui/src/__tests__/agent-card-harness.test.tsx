import { fireEvent, render, screen } from "@testing-library/react";
import AgentsPage from "@/app/(main)/agents/page";
import { AgentCard } from "@/components/agents/agent-card";
import type { Agent } from "@/lib/api/types";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    prefetch: _prefetch,
    ...props
  }: React.AnchorHTMLAttributes<HTMLAnchorElement> & { prefetch?: boolean }) => (
    <a {...props}>{children}</a>
  ),
}));

jest.mock("next/navigation", () => ({
  usePathname: () => "/agents",
  useRouter: () => ({
    push: jest.fn(),
    prefetch: jest.fn(),
    replace: jest.fn(),
    back: jest.fn(),
  }),
}));

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ locale: "en" }),
}));

jest.mock("@/components/chat/streamdown-message", () => ({
  InlineStreamdownMessage: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

const mockAgents: Agent[] = [];
let mockAgentsLoading = false;

jest.mock("@/hooks", () => ({
  usePageTitle: () => {},
  useAgents: () => ({ data: mockAgents, isLoading: mockAgentsLoading, error: null }),
  useCapabilities: () => ({ data: [] }),
  useAgentExamples: () => ({ data: [], isLoading: false, error: null }),
  useImportAgent: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useImportAgentExample: () => ({ mutateAsync: jest.fn(), isPending: false }),
}));

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "agent_019fda100f037c008024046d6b3d74c0",
    name: "researcher",
    display_name: "Researcher",
    description: "Researches difficult questions",
    system_prompt: "Research carefully",
    harness_id: "harness_generic",
    default_model_id: null,
    tags: [],
    capabilities: [],
    status: "active",
    created_at: "2026-08-08T00:00:00Z",
    updated_at: "2026-08-08T00:00:00Z",
    archived_at: null,
    deleted_at: null,
    effective_harness: {
      id: "harness_generic",
      name: "generic",
      display_name: "Generic",
      source: "explicit",
      status: "active",
    },
    ...overrides,
  };
}

describe("AgentCard harness metadata", () => {
  afterEach(() => {
    mockAgents.splice(0);
    mockAgentsLoading = false;
  });

  it("links an explicitly selected effective harness", () => {
    render(<AgentCard agent={agent()} />);

    expect(screen.getByRole("link", { name: "Harness: Generic (explicit)" })).toHaveAttribute(
      "href",
      "/harnesses/harness_generic",
    );
    expect(screen.getByText("Explicit")).toBeInTheDocument();
  });

  it("labels a harness inherited from the organization default", () => {
    render(
      <AgentCard
        agent={agent({
          effective_harness: {
            id: "harness_generic",
            name: "generic",
            display_name: "Generic",
            source: "organization_default",
            status: "active",
          },
        })}
      />,
    );

    expect(
      screen.getByRole("link", { name: "Harness: Generic (organization default)" }),
    ).toHaveAttribute("href", "/harnesses/harness_generic");
    expect(screen.getByText("Org default")).toBeInTheDocument();
  });

  it("renders an unresolved harness honestly without a detail link", () => {
    render(
      <AgentCard
        agent={agent({
          effective_harness: {
            id: "harness_missing",
            name: null,
            display_name: null,
            source: "explicit",
            status: "unresolved",
          },
        })}
      />,
    );

    expect(screen.getByText("unavailable harness")).toHaveAttribute("title", "harness_missing");
    expect(screen.queryByRole("link", { name: /Harness: unavailable/ })).not.toBeInTheDocument();
  });

  it("keeps harness metadata visible when the collection switches to list view", () => {
    mockAgents.push(
      agent({
        effective_harness: {
          id: "harness_long",
          name: "long-harness",
          display_name: "A deliberately long harness name that must stay within the card",
          source: "organization_default",
          status: "active",
        },
      }),
    );

    render(<AgentsPage />);
    fireEvent.click(screen.getByRole("button", { name: "List view" }));

    const harnessLink = screen.getByRole("link", {
      name: /Harness: A deliberately long harness name.*organization default/,
    });
    expect(harnessLink).toHaveClass("truncate");
    expect(harnessLink.closest('[data-slot="entity-card-detail"]')).toBeInTheDocument();
  });

  it("keeps the collection loading state independent of card metadata", () => {
    mockAgentsLoading = true;

    const { container } = render(<AgentsPage />);

    expect(container.querySelectorAll('[data-slot="skeleton"]')).toHaveLength(18);
    expect(container.querySelector('[data-slot="entity-card-detail"]')).not.toBeInTheDocument();
  });
});
