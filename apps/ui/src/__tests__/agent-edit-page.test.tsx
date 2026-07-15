import { render, screen, act, fireEvent } from "@testing-library/react";
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
const mockUseHarnesses = jest.fn();

jest.mock("@/hooks", () => ({
  useAgent: (...args: unknown[]) => mockUseAgent(...args),
  useUpdateAgent: () => mockUseUpdateAgent(),
  useDeleteAgent: () => mockUseDeleteAgent(),
  useDestroyAgent: () => mockUseDestroyAgent(),
  useCapabilities: () => mockUseCapabilities(),
  useAgentNameAvailability: (...args: unknown[]) => mockUseAgentNameAvailability(...args),
  useHarnesses: () => mockUseHarnesses(),
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
  harness_id: "harness_test",
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
    mockUseHarnesses.mockReturnValue({
      data: [{ id: "harness_test", name: "generic", display_name: "Generic" }],
      isLoading: false,
    });
    mockUseAgentNameAvailability.mockReturnValue({
      isChecking: false,
      isAvailable: true,
      error: null,
    });
  });

  it("renders the form when tags is null", async () => {
    await renderWithSuspense({ agentId: "agent_123" });

    expect(screen.getByRole("heading", { name: "Test Agent" })).toBeInTheDocument();
    expect(screen.getByText("Editing")).toBeInTheDocument();
    expect(screen.getByLabelText("Tags")).toHaveValue("");
    expect(screen.getByDisplayValue("test-agent")).toBeInTheDocument();
  });

  it("omits network_access from the update when not edited", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateAgent.mockReturnValue({ mutateAsync, isPending: false });

    await renderWithSuspense({ agentId: "agent_123" });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Save changes/ }));
    });

    expect(mutateAsync).toHaveBeenCalledTimes(1);
    expect(mutateAsync.mock.calls[0][0].request).not.toHaveProperty("network_access");
  });

  it("includes parsed network_access in the update when edited", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateAgent.mockReturnValue({ mutateAsync, isPending: false });

    await renderWithSuspense({ agentId: "agent_123" });
    fireEvent.change(screen.getByLabelText("Allowed hosts"), {
      target: { value: "api.example.com\n*.github.com" },
    });
    fireEvent.change(screen.getByLabelText("Blocked hosts"), {
      target: { value: "internal.corp" },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Save changes/ }));
    });

    expect(mutateAsync).toHaveBeenCalledTimes(1);
    expect(mutateAsync.mock.calls[0][0].request.network_access).toEqual({
      allowed: ["api.example.com", "*.github.com"],
      blocked: ["internal.corp"],
    });
  });

  it("sends an empty network_access object when an existing list is cleared", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateAgent.mockReturnValue({ mutateAsync, isPending: false });
    mockUseAgent.mockReturnValue({
      data: { ...malformedAgent, network_access: { allowed: ["api.example.com"] } },
      isLoading: false,
    });

    await renderWithSuspense({ agentId: "agent_123" });
    expect(screen.getByLabelText("Allowed hosts")).toHaveValue("api.example.com");

    fireEvent.change(screen.getByLabelText("Allowed hosts"), { target: { value: "" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Save changes/ }));
    });

    expect(mutateAsync).toHaveBeenCalledTimes(1);
    // {} clears the layer server-side (omitting would leave it unchanged).
    expect(mutateAsync.mock.calls[0][0].request.network_access).toEqual({});
  });
});
