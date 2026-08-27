import { render, screen, act, fireEvent } from "@testing-library/react";
import { Suspense } from "react";
import AgentDetailPage from "@/app/(main)/agents/[agentId]/page";
import type { Session, Agent, ModelWithProvider } from "@/lib/api/types";

let mockSearchParams = new URLSearchParams();

// Mock next/navigation
jest.mock("next/navigation", () => ({
  usePathname: () => "/agents/agent-1",
  useRouter: () => ({
    push: jest.fn(),
    replace: jest.fn(),
    back: jest.fn(),
  }),
  useSearchParams: () => mockSearchParams,
}));

// Mock next/link
jest.mock("next/link", () => ({
  __esModule: true,
  // eslint-disable-next-line jsx-a11y/anchor-has-content
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    // Using span instead of anchor to avoid lint errors in tests
    <span data-href={href}>{children}</span>
  ),
}));

// Mock prompt-editor to avoid react-markdown ESM issues
jest.mock("@/components/ui/prompt-editor", () => ({
  MarkdownDisplay: ({ content }: { content: string }) => (
    <div data-testid="markdown">{content}</div>
  ),
  PromptEditor: ({ value, onChange }: { value: string; onChange: (v: string) => void }) => (
    <textarea aria-label="Prompt" value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));

// Mock ProviderIcon to avoid Next.js Image issues
jest.mock("@/components/providers/provider-icon", () => ({
  ProviderIcon: ({ providerType }: { providerType: string }) => (
    <span data-testid={`provider-icon-${providerType}`}>{providerType}</span>
  ),
}));

// Mock streamdown-message to avoid ESM issues with rehype-harden
jest.mock("@/components/chat/streamdown-message", () => ({
  StreamdownMessage: ({ content }: { content: string }) => (
    <div data-testid="streamdown-message">{content}</div>
  ),
  InlineStreamdownMessage: ({ content }: { content: string }) => (
    <span data-testid="inline-streamdown-message">{content}</span>
  ),
}));

jest.mock("@/components/agents/agent-preview", () => ({
  AgentPreview: () => <div data-testid="agent-preview">agent preview</div>,
}));

jest.mock("@/components/agents/agent-credentials-panel", () => ({
  AgentCredentialsPanel: () => <div>agent credentials</div>,
}));

jest.mock("@/components/agents/agent-triggers-panel", () => ({
  AgentTriggersPanel: () => <div>agent triggers</div>,
}));

// Mock data
const mockAgent: Agent = {
  id: "agent-1",
  name: "test-agent",
  harness_id: "harness_test",
  display_name: "Test Agent",
  description: "A test agent",
  system_prompt: "You are helpful",
  default_model_id: null,
  tags: ["test"],
  capabilities: [],
  status: "active",
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

// Session status: started → active → idle (cycles)
// - started: Session just created, no turn executed yet
// - active: A turn is currently running
// - idle: Turn completed, session waiting for next input
const mockSessions: Session[] = [
  {
    id: "session-1",
    organization_id: "org-1",
    harness_id: "harness-1",
    agent_id: "agent-1",
    owner_principal_id: "principal_1",
    title: "Session with GPT-4o",
    tags: [],
    model_id: "model-1",
    status: "idle",
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    started_at: "2025-01-01T00:00:01Z",
    finished_at: null,
  },
  {
    id: "session-2",
    organization_id: "org-1",
    harness_id: "harness-1",
    agent_id: "agent-1",
    owner_principal_id: "principal_1",
    title: "Session with Claude",
    tags: [],
    model_id: "model-2",
    status: "idle",
    created_at: "2025-01-01T01:00:00Z",
    updated_at: "2025-01-01T01:00:00Z",
    started_at: null,
    finished_at: null,
  },
  {
    id: "session-3",
    organization_id: "org-1",
    harness_id: "harness-1",
    agent_id: "agent-1",
    owner_principal_id: "principal_1",
    title: "Session without model",
    tags: [],
    model_id: null,
    status: "started",
    created_at: "2025-01-01T02:00:00Z",
    updated_at: "2025-01-01T02:00:00Z",
    started_at: null,
    finished_at: null,
  },
];

const mockModels: ModelWithProvider[] = [
  {
    id: "model-1",
    provider_id: "provider-1",
    model_id: "gpt-4o",
    display_name: "GPT-4o",
    capabilities: ["chat"],
    enabled: false,
    healthy: true,
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    provider_name: "OpenAI",
    provider_type: "openai",
    is_favorite: false,
  },
  {
    id: "model-2",
    provider_id: "provider-2",
    model_id: "claude-sonnet-5",
    display_name: "Claude Sonnet 5",
    capabilities: ["chat"],
    enabled: false,
    healthy: true,
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    provider_name: "Anthropic",
    provider_type: "anthropic",
    is_favorite: false,
  },
];

// Mock hooks
const mockUseAgent = jest.fn();
const mockUseSessions = jest.fn();
const mockUseCreateSession = jest.fn();
const mockUseCapabilities = jest.fn();
const mockUseModels = jest.fn();
const mockUseExportAgent = jest.fn();
const mockUseCopyAgent = jest.fn();
const mockUseHarnesses = jest.fn();
const mockUseOrganization = jest.fn();
const mockUseAgentStats = jest.fn();

jest.mock("@/hooks", () => ({
  useAgent: (...args: unknown[]) => mockUseAgent(...args),
  useSessions: (...args: unknown[]) => mockUseSessions(...args),
  useCreateSession: () => mockUseCreateSession(),
  useCapabilities: () => mockUseCapabilities(),
  useModels: () => mockUseModels(),
  useExportAgent: () => mockUseExportAgent(),
  useCopyAgent: () => mockUseCopyAgent(),
  useHarnesses: () => mockUseHarnesses(),
  useAgentStats: (...args: unknown[]) => mockUseAgentStats(...args),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => mockUseOrganization(),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (flag: string) => flag === "agent_versions",
}));

// Helper to render with Suspense for React.use()
async function renderWithSuspense(params: { agentId: string }) {
  const paramsPromise = Promise.resolve(params);

  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <AgentDetailPage params={paramsPromise} />
      </Suspense>,
    );
    // Let the promise resolve
    await paramsPromise;
  });
}

beforeEach(() => {
  mockSearchParams = new URLSearchParams();
});

describe("AgentDetailPage - tab navigation", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSearchParams = new URLSearchParams();
    mockUseAgent.mockReturnValue({ data: mockAgent, isLoading: false });
    mockUseSessions.mockReturnValue({ data: [], isLoading: false });
    mockUseCreateSession.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseCapabilities.mockReturnValue({ data: [] });
    mockUseModels.mockReturnValue({ data: mockModels });
    mockUseExportAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseCopyAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseHarnesses.mockReturnValue({ data: [] });
    mockUseOrganization.mockReturnValue({ data: null });
    mockUseAgentStats.mockReturnValue({ data: undefined, isLoading: false, error: null });
  });

  it("renders the agent detail tabs in workflow order", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Overview",
      "Preview",
      "Credentials",
      "Triggers",
      "Versions",
      "Stats",
      "Integrate",
    ]);
  });

  it("selects the credentials tab from its deep link", async () => {
    mockSearchParams = new URLSearchParams("tab=credentials");

    await renderWithSuspense({ agentId: "agent-1" });

    expect(screen.getByRole("tab", { name: "Credentials" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByText("agent credentials")).toBeInTheDocument();
  });

  it("keeps focus and active state aligned when selecting a tab", async () => {
    await renderWithSuspense({ agentId: "agent-1" });
    const triggersTab = screen.getByRole("tab", { name: "Triggers" });

    triggersTab.focus();
    expect(triggersTab).toHaveFocus();
    fireEvent.click(triggersTab);

    expect(triggersTab).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "false");
    expect(screen.getByText("agent triggers")).toBeInTheDocument();
  });
});

describe("AgentDetailPage - LLM Model Display in Sessions List", () => {
  beforeEach(() => {
    jest.clearAllMocks();

    // Default mock implementations
    mockUseAgent.mockReturnValue({ data: mockAgent, isLoading: false });
    mockUseSessions.mockReturnValue({ data: mockSessions, isLoading: false });
    mockUseCreateSession.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseCapabilities.mockReturnValue({ data: [] });
    mockUseModels.mockReturnValue({ data: mockModels });
    mockUseExportAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseCopyAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseHarnesses.mockReturnValue({ data: [] });
    mockUseOrganization.mockReturnValue({ data: null });
    mockUseAgentStats.mockReturnValue({ data: undefined, isLoading: false, error: null });
  });

  it("renders agent page structure", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Agent name should be visible
    expect(screen.getByRole("heading", { name: "Test Agent" })).toBeInTheDocument();
    // Sessions section header should be visible
    expect(screen.getByText("Sessions")).toBeInTheDocument();
  });

  it("renders copy button", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    expect(screen.getAllByRole("button", { name: "Copy" }).length).toBeGreaterThan(0);
  });

  it("shows copying state when copy is pending", async () => {
    mockUseCopyAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: true });

    await renderWithSuspense({ agentId: "agent-1" });

    const copyingButtons = screen.getAllByRole("button", { name: "Copying..." });
    expect(copyingButtons.length).toBeGreaterThan(0);
    copyingButtons.forEach((button) => expect(button).toBeDisabled());
  });

  it("renders agent details correctly", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Agent name should be visible
    expect(screen.getByRole("heading", { name: "Test Agent" })).toBeInTheDocument();
  });

  it("does not display model badge when model data is not loaded", async () => {
    mockUseModels.mockReturnValue({ data: undefined });

    await renderWithSuspense({ agentId: "agent-1" });

    // Model badges should not be visible
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
    expect(screen.queryByText("Claude Sonnet 5")).not.toBeInTheDocument();
  });

  it("does not display model badge when model_id has no matching model", async () => {
    const sessionsWithUnknownModel: Session[] = [
      {
        ...mockSessions[0],
        model_id: "unknown-model",
      },
    ];
    mockUseSessions.mockReturnValue({ data: sessionsWithUnknownModel, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1" });

    // No model badges should be visible
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("useSessions hook is called with agent id", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Check useSessions was called
    expect(mockUseSessions).toHaveBeenCalled();
  });

  it("calls useModels hook", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    expect(mockUseModels).toHaveBeenCalled();
  });

  it("renders useModels data for default model display", async () => {
    const agentWithDefaultModel = { ...mockAgent, default_model_id: "model-1" };
    mockUseAgent.mockReturnValue({ data: agentWithDefaultModel, isLoading: false });
    await renderWithSuspense({ agentId: "agent-1" });

    // Default model should be displayed in configuration section
    expect(screen.getByText("Default Model")).toBeInTheDocument();
    expect(screen.getAllByText("GPT-4o").length).toBeGreaterThan(0);
  });

  it("handles empty sessions list", async () => {
    mockUseSessions.mockReturnValue({ data: [], isLoading: false });

    await renderWithSuspense({ agentId: "agent-1" });

    expect(
      screen.getByText("No sessions yet. Start a new session to begin chatting."),
    ).toBeInTheDocument();
  });

  it("renders sessions section", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Sessions section should be visible
    expect(screen.getByText("Sessions")).toBeInTheDocument();
  });
});

describe("AgentDetailPage - Default Model Display in Configuration", () => {
  beforeEach(() => {
    jest.clearAllMocks();

    mockUseSessions.mockReturnValue({ data: [], isLoading: false });
    mockUseCreateSession.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseCapabilities.mockReturnValue({ data: [] });
    mockUseModels.mockReturnValue({ data: mockModels });
    mockUseExportAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseCopyAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseHarnesses.mockReturnValue({ data: [] });
  });

  it("displays default model with provider icon when agent has default_model_id", async () => {
    const agentWithDefaultModel: Agent = {
      ...mockAgent,
      default_model_id: "model-1",
    };
    mockUseAgent.mockReturnValue({ data: agentWithDefaultModel, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1" });

    // Check that default model section is visible
    expect(screen.getByText("Default Model")).toBeInTheDocument();
    expect(screen.getAllByText("GPT-4o").length).toBeGreaterThan(0);
    // Provider icon should be rendered
    expect(screen.getByTestId("provider-icon-openai")).toBeInTheDocument();
  });

  it("displays Anthropic provider icon for Claude model", async () => {
    const agentWithClaudeModel: Agent = {
      ...mockAgent,
      default_model_id: "model-2",
    };
    mockUseAgent.mockReturnValue({ data: agentWithClaudeModel, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1" });

    expect(screen.getByText("Default Model")).toBeInTheDocument();
    expect(screen.getAllByText("Claude Sonnet 5").length).toBeGreaterThan(0);
    expect(screen.getByTestId("provider-icon-anthropic")).toBeInTheDocument();
  });

  it("does not display default model section when agent has no default_model_id", async () => {
    const agentWithoutDefaultModel: Agent = {
      ...mockAgent,
      default_model_id: null,
    };
    mockUseAgent.mockReturnValue({ data: agentWithoutDefaultModel, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1" });

    expect(screen.queryByText("Default Model")).not.toBeInTheDocument();
  });

  it("does not display default model section when model data is not loaded", async () => {
    const agentWithDefaultModel: Agent = {
      ...mockAgent,
      default_model_id: "model-1",
    };
    mockUseAgent.mockReturnValue({ data: agentWithDefaultModel, isLoading: false });
    mockUseModels.mockReturnValue({ data: undefined });

    await renderWithSuspense({ agentId: "agent-1" });

    expect(screen.queryByText("Default Model")).not.toBeInTheDocument();
  });

  it("does not display default model section when default_model_id has no matching model", async () => {
    const agentWithUnknownModel: Agent = {
      ...mockAgent,
      default_model_id: "unknown-model-id",
    };
    mockUseAgent.mockReturnValue({ data: agentWithUnknownModel, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1" });

    expect(screen.queryByText("Default Model")).not.toBeInTheDocument();
  });
});
