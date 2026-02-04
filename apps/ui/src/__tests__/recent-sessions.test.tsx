import { render, screen } from "@testing-library/react";
import { RecentSessions } from "@/components/dashboard/recent-sessions";
import type { Session, Agent, LlmModelWithProvider } from "@/lib/api/types";

// Helper to create a test session
function createSession(overrides?: Partial<Session>): Session {
  return {
    id: "session-1",
    agent_id: "agent-1",
    title: "Test Session",
    tags: [],
    model_id: null,
    status: "started",
    created_at: "2025-01-01T00:00:00Z",
    started_at: null,
    finished_at: null,
    ...overrides,
  };
}

// Helper to create a test agent
function createAgent(overrides?: Partial<Agent>): Agent {
  return {
    id: "agent-1",
    name: "Test Agent",
    description: null,
    system_prompt: "You are helpful",
    default_model_id: null,
    tags: [],
    status: "active",
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    ...overrides,
  };
}

// Helper to create a test LLM model
function createLlmModel(overrides?: Partial<LlmModelWithProvider>): LlmModelWithProvider {
  return {
    id: "model-1",
    provider_id: "provider-1",
    model_id: "gpt-4o",
    display_name: "GPT-4o",
    capabilities: ["chat"],
    is_default: false,
    status: "active",
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    provider_name: "OpenAI",
    provider_type: "openai",
    ...overrides,
  };
}

describe("RecentSessions", () => {
  describe("rendering", () => {
    it("renders Recent Sessions header", () => {
      const sessions = [createSession()];
      const agents = [createAgent()];

      render(<RecentSessions sessions={sessions} agents={agents} />);

      expect(screen.getByText("Recent Sessions")).toBeInTheDocument();
    });

    it("displays session title", () => {
      const session = createSession({ title: "My Test Session" });
      const agent = createAgent();

      render(<RecentSessions sessions={[session]} agents={[agent]} />);

      expect(screen.getByText("My Test Session")).toBeInTheDocument();
    });

    it("displays agent name when agent exists", () => {
      const session = createSession();
      const agent = createAgent({ name: "Code Assistant" });

      render(<RecentSessions sessions={[session]} agents={[agent]} />);

      expect(screen.getByText("Code Assistant")).toBeInTheDocument();
    });
  });

  describe("model display", () => {
    it("displays model name when session has a model_id and model data is provided", () => {
      const model = createLlmModel({ id: "model-123", display_name: "Claude 3.5 Sonnet" });
      const session = createSession({ model_id: "model-123" });
      const agent = createAgent();

      render(<RecentSessions sessions={[session]} agents={[agent]} models={[model]} />);

      expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();
    });

    it("does not display model when session has no model_id", () => {
      const session = createSession({ model_id: null });
      const agent = createAgent();

      render(<RecentSessions sessions={[session]} agents={[agent]} />);

      expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
    });

    it("does not display model when session has model_id but no matching model data", () => {
      const session = createSession({ model_id: "unknown-model" });
      const agent = createAgent();

      render(<RecentSessions sessions={[session]} agents={[agent]} models={[]} />);

      // No model should be displayed
      expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
    });

    it("displays multiple sessions with different models", () => {
      const model1 = createLlmModel({ id: "model-1", display_name: "GPT-4o" });
      const model2 = createLlmModel({ id: "model-2", display_name: "Claude 3.5 Sonnet" });

      const session1 = createSession({ id: "s1", model_id: "model-1", title: "Session 1" });
      const session2 = createSession({ id: "s2", model_id: "model-2", title: "Session 2" });
      const session3 = createSession({ id: "s3", model_id: null, title: "Session 3" });

      const agent = createAgent();

      render(
        <RecentSessions
          sessions={[session1, session2, session3]}
          agents={[agent]}
          models={[model1, model2]}
        />,
      );

      expect(screen.getByText("GPT-4o")).toBeInTheDocument();
      expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();
    });
  });

  describe("status display", () => {
    it("shows Running badge for active sessions", () => {
      const session = createSession({ status: "active" });
      const agent = createAgent();

      render(<RecentSessions sessions={[session]} agents={[agent]} />);

      expect(screen.getByText("Running")).toBeInTheDocument();
    });

    it("shows Idle badge for idle sessions", () => {
      const session = createSession({ status: "idle" });
      const agent = createAgent();

      render(<RecentSessions sessions={[session]} agents={[agent]} />);

      expect(screen.getByText("Idle")).toBeInTheDocument();
    });

    it("shows New badge for started sessions", () => {
      const session = createSession({ status: "started" });
      const agent = createAgent();

      render(<RecentSessions sessions={[session]} agents={[agent]} />);

      expect(screen.getByText("New")).toBeInTheDocument();
    });
  });

  describe("empty state", () => {
    it("shows empty message when no sessions", () => {
      render(<RecentSessions sessions={[]} agents={[]} />);

      expect(
        screen.getByText("No sessions yet. Create an agent and start a session to begin."),
      ).toBeInTheDocument();
    });
  });

  describe("backwards compatibility", () => {
    it("works without models prop (defaults to empty array)", () => {
      const session = createSession({ model_id: "some-model" });
      const agent = createAgent();

      // Should not throw when models prop is omitted
      render(<RecentSessions sessions={[session]} agents={[agent]} />);

      // Session should still render
      expect(screen.getByText("Test Session")).toBeInTheDocument();
    });
  });
});
