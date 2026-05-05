import { render, screen, act } from "@testing-library/react";
import { Suspense } from "react";
import EditAgentPage from "@/app/(main)/agents/[agentId]/edit/page";
import type { Agent } from "@/lib/api/types";

const push = jest.fn();
const replace = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({
    push,
    replace,
    back: jest.fn(),
  }),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    href,
    ...props
  }: {
    children: React.ReactNode;
    href: string;
    [key: string]: unknown;
  }) => (
    // eslint-disable-next-line @next/next/no-html-link-for-pages
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

jest.mock("@/components/ui/prompt-editor", () => ({
  PromptEditor: ({ value, onChange }: { value: string; onChange: (value: string) => void }) => (
    <textarea aria-label="System prompt" value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));

jest.mock("@/components/agents/capability-selector", () => ({
  CapabilitySelector: () => <div data-testid="capability-selector" />,
}));

jest.mock("@/components/agents/agent-preview", () => ({
  AgentPreview: () => <div data-testid="agent-preview" />,
}));

jest.mock("@/components/initial-files-editor", () => ({
  InitialFilesEditor: () => <div data-testid="initial-files-editor" />,
}));

jest.mock("@/components/models/model-picker", () => ({
  ModelPicker: ({
    value,
    onChange,
  }: {
    value?: string | null;
    onChange: (value: string) => void;
  }) => (
    <input
      aria-label="Default model"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value)}
    />
  ),
}));

const mockUseAgent = jest.fn();
const mockUseUpdateAgent = jest.fn();
const mockUseDeleteAgent = jest.fn();
const mockUseDestroyAgent = jest.fn();
const mockUseCapabilities = jest.fn();
const mockUseAgentNameAvailability = jest.fn();

jest.mock("@/hooks", () => ({
  useAgent: (...args: unknown[]) => mockUseAgent(...args),
  useUpdateAgent: () => mockUseUpdateAgent(),
  useDeleteAgent: () => mockUseDeleteAgent(),
  useDestroyAgent: () => mockUseDestroyAgent(),
  useCapabilities: () => mockUseCapabilities(),
  useAgentNameAvailability: (...args: unknown[]) => mockUseAgentNameAvailability(...args),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({
    can: () => true,
  }),
}));

const malformedAgent: Agent = {
  id: "agent_123",
  name: "test-agent",
  display_name: "Test Agent",
  description: "Agent description",
  system_prompt: "You are helpful.",
  default_model_id: null,
  tags: null as unknown as string[],
  capabilities: [],
  initial_files: [],
  status: "active",
  created_at: "2026-04-19T10:00:00Z",
  updated_at: "2026-04-19T10:00:00Z",
  archived_at: null,
  deleted_at: null,
};

async function renderWithSuspense(params: { agentId: string }) {
  const paramsPromise = Promise.resolve(params);

  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <EditAgentPage params={paramsPromise} />
      </Suspense>,
    );
    await paramsPromise;
  });
}

describe("EditAgentPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAgent.mockReturnValue({ data: malformedAgent, isLoading: false });
    mockUseUpdateAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseDeleteAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseDestroyAgent.mockReturnValue({ mutateAsync: jest.fn(), isPending: false });
    mockUseCapabilities.mockReturnValue({ data: [], isLoading: false });
    mockUseAgentNameAvailability.mockReturnValue({
      isChecking: false,
      isAvailable: true,
      error: null,
    });
  });

  it("renders the form when tags is null", async () => {
    await renderWithSuspense({ agentId: "agent_123" });

    expect(screen.getByRole("heading", { name: "Edit Agent" })).toBeInTheDocument();
    expect(screen.getByLabelText("Tags")).toHaveValue("");
    expect(screen.getByDisplayValue("test-agent")).toBeInTheDocument();
  });
});
