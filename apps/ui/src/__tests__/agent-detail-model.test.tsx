import { render, screen, act } from "@testing-library/react";
import { Suspense } from "react";
import AgentDetailPage from "@/app/(main)/agents/[agentId]/page";
import type { Session, Agent, LlmModelWithProvider } from "@/lib/api/types";

// Mock next/navigation
jest.mock("next/navigation", () => ({
  useRouter: () => ({
    push: jest.fn(),
    replace: jest.fn(),
    back: jest.fn(),
  }),
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
  MarkdownDisplay: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
  PromptEditor: ({ value, onChange }: { value: string; onChange: (v: string) => void }) => (
    <textarea value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));

// Mock ProviderIcon to avoid Next.js Image issues
jest.mock("@/components/providers/provider-icon", () => ({
  ProviderIcon: ({ providerType }: { providerType: string }) => (
    <span data-testid={`provider-icon-${providerType}`}>{providerType}</span>
  ),
}));

// Mock data
const mockAgent: Agent = {
  id: "agent-1",
  name: "Test Agent",
  description: "A test agent",
  system_prompt: "You are helpful",
  default_model_id: null,
  tags: ["test"],
  status: "active",
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
};

const mockSessions: Session[] = [
  {
    id: "session-1",
    agent_id: "agent-1",
    title: "Session with GPT-4o",
    tags: [],
    model_id: "model-1",
    status: "completed",
    created_at: "2025-01-01T00:00:00Z",
    started_at: "2025-01-01T00:00:01Z",
    finished_at: "2025-01-01T00:01:00Z",
  },
  {
    id: "session-2",
    agent_id: "agent-1",
    title: "Session with Claude",
    tags: [],
    model_id: "model-2",
    status: "pending",
    created_at: "2025-01-01T01:00:00Z",
    started_at: null,
    finished_at: null,
  },
  {
    id: "session-3",
    agent_id: "agent-1",
    title: "Session without model",
    tags: [],
    model_id: null,
    status: "pending",
    created_at: "2025-01-01T02:00:00Z",
    started_at: null,
    finished_at: null,
  },
];

const mockLlmModels: LlmModelWithProvider[] = [
  {
    id: "model-1",
    provider_id: "provider-1",
    model_id: "gpt-4o",
    display_name: "GPT-4o",
    capabilities: ["chat"],
    context_window: 128000,
    is_default: false,
    status: "active",
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    provider_name: "OpenAI",
    provider_type: "openai",
  },
  {
    id: "model-2",
    provider_id: "provider-2",
    model_id: "claude-3-5-sonnet",
    display_name: "Claude 3.5 Sonnet",
    capabilities: ["chat"],
    context_window: 200000,
    is_default: false,
    status: "active",
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    provider_name: "Anthropic",
    provider_type: "anthropic",
  },
];

// Mock hooks
const mockUseAgent = jest.fn();
const mockUseSessions = jest.fn();
const mockUseCreateSession = jest.fn();
const mockUseAgentCapabilities = jest.fn();
const mockUseCapabilities = jest.fn();
const mockUseLlmModels = jest.fn();

jest.mock("@/hooks", () => ({
  useAgent: (...args: unknown[]) => mockUseAgent(...args),
  useSessions: (...args: unknown[]) => mockUseSessions(...args),
  useCreateSession: () => mockUseCreateSession(),
  useAgentCapabilities: (...args: unknown[]) => mockUseAgentCapabilities(...args),
  useCapabilities: () => mockUseCapabilities(),
  useLlmModels: () => mockUseLlmModels(),
}));

// Helper to render with Suspense for React.use()
async function renderWithSuspense(params: { agentId: string }) {
  const paramsPromise = Promise.resolve(params);

  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <AgentDetailPage params={paramsPromise} />
      </Suspense>
    );
    // Let the promise resolve
    await paramsPromise;
  });
}

describe("AgentDetailPage - LLM Model Display in Sessions List", () => {
  beforeEach(() => {
    jest.clearAllMocks();

    // Default mock implementations
    mockUseAgent.mockReturnValue({ data: mockAgent, isLoading: false });
    mockUseSessions.mockReturnValue({ data: mockSessions, isLoading: false });
    mockUseCreateSession.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseAgentCapabilities.mockReturnValue({ data: [], isLoading: false });
    mockUseCapabilities.mockReturnValue({ data: [] });
    mockUseLlmModels.mockReturnValue({ data: mockLlmModels });
  });

  it("displays model badge for sessions with model_id", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Model badges should be visible
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();
  });

  it("does not display model badge for sessions without model_id", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Count the model badges - should only be 2 (not 3)
    const gpt4oBadges = screen.getAllByText("GPT-4o");
    const claudeBadges = screen.getAllByText("Claude 3.5 Sonnet");

    expect(gpt4oBadges).toHaveLength(1);
    expect(claudeBadges).toHaveLength(1);
  });

  it("does not display model badge when model data is not loaded", async () => {
    mockUseLlmModels.mockReturnValue({ data: undefined });

    await renderWithSuspense({ agentId: "agent-1" });

    // Model badges should not be visible
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
    expect(screen.queryByText("Claude 3.5 Sonnet")).not.toBeInTheDocument();
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

  it("displays Completed badge alongside model badge", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Both model and completion status should be visible for completed session
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Completed")).toBeInTheDocument();
  });

  it("calls useLlmModels hook", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    expect(mockUseLlmModels).toHaveBeenCalled();
  });

  it("creates model map with correct display names", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Verify correct model names are displayed
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();
  });

  it("handles empty sessions list", async () => {
    mockUseSessions.mockReturnValue({ data: [], isLoading: false });

    await renderWithSuspense({ agentId: "agent-1" });

    expect(
      screen.getByText("No sessions yet. Start a new session to begin chatting.")
    ).toBeInTheDocument();
  });

  it("displays provider icons for sessions with models", async () => {
    await renderWithSuspense({ agentId: "agent-1" });

    // Provider icons should be rendered for sessions with models
    expect(screen.getByTestId("provider-icon-openai")).toBeInTheDocument();
    expect(screen.getByTestId("provider-icon-anthropic")).toBeInTheDocument();
  });
});

describe("AgentDetailPage - Default Model Display in Configuration", () => {
  beforeEach(() => {
    jest.clearAllMocks();

    mockUseSessions.mockReturnValue({ data: [], isLoading: false });
    mockUseCreateSession.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseAgentCapabilities.mockReturnValue({ data: [], isLoading: false });
    mockUseCapabilities.mockReturnValue({ data: [] });
    mockUseLlmModels.mockReturnValue({ data: mockLlmModels });
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
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
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
    expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();
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
    mockUseLlmModels.mockReturnValue({ data: undefined });

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
