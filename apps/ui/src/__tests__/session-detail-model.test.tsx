import { render, screen } from "@testing-library/react";
import { SessionCard } from "@/components/session/session-card";
import type { Session, ModelWithProvider } from "@/lib/api/types";

// Mock next/link - using span with data-href to avoid linting issues
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <span data-testid="session-link" data-href={href}>
      {children}
    </span>
  ),
}));

// Mock data
const mockSession: Session = {
  id: "session-1",
  organization_id: "org_default",
  harness_id: "harness-default",
  agent_id: "agent-1",
  owner_principal_id: "principal-1",
  title: "Test Session",
  tags: [],
  model_id: "model-1",
  status: "idle",
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:02Z",
  started_at: "2025-01-01T00:00:01Z",
  finished_at: null,
};

const mockModel: ModelWithProvider = {
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
};

describe("SessionCard - LLM Model Display", () => {
  it("displays LLM model badge when session has model", async () => {
    render(<SessionCard session={mockSession} model={mockModel} />);

    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
  });

  it("does not display model badge when model is not provided", async () => {
    render(<SessionCard session={mockSession} />);

    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("displays model badge alongside status badge", async () => {
    render(<SessionCard session={mockSession} model={mockModel} />);

    // Both model and status should be visible
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Idle")).toBeInTheDocument();
  });

  it("displays different model names correctly", async () => {
    const claudeModel: ModelWithProvider = {
      ...mockModel,
      id: "model-2",
      display_name: "Claude Sonnet 5",
      provider_name: "Anthropic",
      provider_type: "anthropic",
    };

    render(<SessionCard session={mockSession} model={claudeModel} />);

    expect(screen.getByText("Claude Sonnet 5")).toBeInTheDocument();
  });

  it("links to org-level session URL", async () => {
    render(<SessionCard session={mockSession} model={mockModel} />);

    // Check that the link points to the org-level session path
    const link = screen.getByTestId("session-link");
    expect(link).toHaveAttribute("data-href", "/sessions/session-1/transcript");
  });

  it("displays agent name when provided", async () => {
    render(<SessionCard session={mockSession} agentName="Test Agent" model={mockModel} />);

    expect(screen.getByText("Test Agent")).toBeInTheDocument();
  });
});
