import { render, screen, act } from "@testing-library/react";
import { Suspense } from "react";
import SessionDetailPage from "@/app/(main)/agents/[agentId]/sessions/[sessionId]/page";
import type { Session, Agent, LlmModelWithProvider } from "@/lib/api/types";

// Mock scrollIntoView for jsdom
Element.prototype.scrollIntoView = jest.fn();

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
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

// Mock data
const mockAgent: Agent = {
  id: "agent-1",
  name: "Test Agent",
  description: null,
  system_prompt: "You are helpful",
  default_model_id: null,
  tags: [],
  status: "active",
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
};

const mockSession: Session = {
  id: "session-1",
  agent_id: "agent-1",
  title: "Test Session",
  tags: [],
  model_id: "model-1",
  status: "pending",
  created_at: "2025-01-01T00:00:00Z",
  started_at: "2025-01-01T00:00:01Z",
  finished_at: null,
};

const mockLlmModel: LlmModelWithProvider = {
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
};

// Mock hooks module
const mockUseAgent = jest.fn();
const mockUseSession = jest.fn();
const mockUseMessages = jest.fn();
const mockUseSendMessage = jest.fn();
const mockUseLlmModel = jest.fn();

jest.mock("@/hooks", () => ({
  useAgent: (...args: unknown[]) => mockUseAgent(...args),
  useSession: (...args: unknown[]) => mockUseSession(...args),
  useMessages: (...args: unknown[]) => mockUseMessages(...args),
  useSendMessage: () => mockUseSendMessage(),
  useLlmModel: (...args: unknown[]) => mockUseLlmModel(...args),
}));

// Helper to render with Suspense for React.use()
async function renderWithSuspense(params: { agentId: string; sessionId: string }) {
  const paramsPromise = Promise.resolve(params);

  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <SessionDetailPage params={paramsPromise} />
      </Suspense>
    );
    // Let the promise resolve
    await paramsPromise;
  });
}

describe("SessionDetailPage - LLM Model Display", () => {
  beforeEach(() => {
    jest.clearAllMocks();

    // Default mock implementations
    mockUseAgent.mockReturnValue({ data: mockAgent, isLoading: false });
    mockUseSession.mockReturnValue({ data: mockSession, isLoading: false });
    mockUseMessages.mockReturnValue({ data: [], isLoading: false });
    mockUseSendMessage.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseLlmModel.mockReturnValue({ data: mockLlmModel, isLoading: false });
  });

  it("displays LLM model badge when session has model_id", async () => {
    await renderWithSuspense({ agentId: "agent-1", sessionId: "session-1" });

    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
  });

  it("calls useLlmModel with session model_id", async () => {
    await renderWithSuspense({ agentId: "agent-1", sessionId: "session-1" });

    // Verify useLlmModel was called with the model_id
    expect(mockUseLlmModel).toHaveBeenCalledWith("model-1");
  });

  it("does not display model badge when session has no model_id", async () => {
    const sessionWithoutModel: Session = { ...mockSession, model_id: null };
    mockUseSession.mockReturnValue({ data: sessionWithoutModel, isLoading: false });
    mockUseLlmModel.mockReturnValue({ data: undefined, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1", sessionId: "session-1" });

    // Model badge should not be present
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("does not display model badge when model data is not loaded yet", async () => {
    mockUseLlmModel.mockReturnValue({ data: undefined, isLoading: true });

    await renderWithSuspense({ agentId: "agent-1", sessionId: "session-1" });

    // Model badge should not be present while loading
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("passes empty string to useLlmModel when session model_id is null", async () => {
    const sessionWithoutModel: Session = { ...mockSession, model_id: null };
    mockUseSession.mockReturnValue({ data: sessionWithoutModel, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1", sessionId: "session-1" });

    // Should pass empty string when model_id is null
    expect(mockUseLlmModel).toHaveBeenCalledWith("");
  });

  it("displays model badge alongside status badge", async () => {
    await renderWithSuspense({ agentId: "agent-1", sessionId: "session-1" });

    // Both model and status should be visible
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Ready")).toBeInTheDocument(); // pending status
  });

  it("displays different model names correctly", async () => {
    const claudeModel: LlmModelWithProvider = {
      ...mockLlmModel,
      id: "model-2",
      display_name: "Claude 3.5 Sonnet",
      provider_name: "Anthropic",
      provider_type: "anthropic",
    };
    const sessionWithClaude: Session = { ...mockSession, model_id: "model-2" };

    mockUseSession.mockReturnValue({ data: sessionWithClaude, isLoading: false });
    mockUseLlmModel.mockReturnValue({ data: claudeModel, isLoading: false });

    await renderWithSuspense({ agentId: "agent-1", sessionId: "session-1" });

    expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();
  });
});
