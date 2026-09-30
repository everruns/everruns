import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { Suspense } from "react";
import AgentPage from "@/app/(main)/agents/[agentId]/page";
import type { Agent, Session } from "@/lib/api/types";

let mockSearchParams = new URLSearchParams();
const push = jest.fn();
const replace = jest.fn();

jest.mock("next/navigation", () => ({
  usePathname: () => "/agents/agent-1",
  useRouter: () => ({ push, replace, back: jest.fn() }),
  useSearchParams: () => mockSearchParams,
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <span data-href={typeof href === "string" ? href : JSON.stringify(href)}>{children}</span>
  ),
}));

jest.mock("@/components/chat/streamdown-message", () => ({
  StreamdownMessage: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="markdown">{children}</div>
  ),
  InlineStreamdownMessage: ({ children }: { children: React.ReactNode }) => <span>{children}</span>,
}));

jest.mock("@/components/agents/agent-preview", () => ({
  AgentPreview: ({ systemPrompt }: { systemPrompt: string }) => (
    <div data-testid="agent-preview">{systemPrompt}</div>
  ),
}));
jest.mock("@/components/agents/agent-credentials-panel", () => ({
  AgentCredentialsPanel: () => <div>agent credentials</div>,
}));
jest.mock("@/components/agents/agent-mcp-panel", () => ({
  AgentMcpPanel: () => <div>agent mcp</div>,
}));
jest.mock("@/components/agents/agent-integrations-panel", () => ({
  AgentIntegrationsPanel: () => <div>agent integrations</div>,
}));
jest.mock("@/components/agents/agent-version-history", () => ({
  AgentVersionHistory: () => <div>agent versions</div>,
}));
jest.mock("@/components/agents/agent-health-check", () => ({
  AgentHealthCheck: () => <div data-testid="agent-health-check" />,
}));
jest.mock("@/components/stats/resource-stats-panel", () => ({
  ResourceStatsPanel: () => <div>agent stats</div>,
}));
jest.mock("@/components/initial-files-editor", () => ({
  InitialFilesEditor: () => <div data-testid="initial-files-editor" />,
}));
jest.mock("@/components/agents/agent-checks", () => ({
  AgentChecks: ({
    systemPrompt,
    onApplyFix,
  }: {
    systemPrompt: string;
    onApplyFix: (start: number, end: number, replacement: string) => void;
  }) => (
    <div data-testid="agent-checks">
      <span data-testid="checks-prompt">{systemPrompt}</span>
      <button type="button" onClick={() => onApplyFix(0, 5, "Fixed")}>
        Apply check fix
      </button>
    </div>
  ),
  applyByteSpanReplacement: (text: string, _start: number, end: number, replacement: string) =>
    replacement + text.slice(end),
}));
jest.mock("@/components/agents/capability-selector", () => ({
  CapabilitySelector: ({
    onChange,
  }: {
    onChange: (selected: Array<{ ref: string; config: Record<string, unknown> }>) => void;
  }) => (
    <div data-testid="capability-selector">
      <button type="button" onClick={() => onChange([{ ref: "memory", config: {} }])}>
        Select memory
      </button>
    </div>
  ),
}));
jest.mock("@/components/models/model-picker", () => ({
  ModelPicker: ({ value, onChange }: { value?: string; onChange: (value: string) => void }) => (
    <input
      aria-label="Default model"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value)}
    />
  ),
}));

const mockAgent: Agent = {
  id: "agent-1",
  name: "test-agent",
  harness_id: "harness_test",
  display_name: "Test Agent",
  description: "A test agent",
  system_prompt: "You are helpful",
  default_model_id: null,
  tags: null as unknown as string[],
  capabilities: [],
  initial_files: [],
  tools: [],
  status: "active",
  session_count: 3,
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

const mockSession: Session = {
  id: "session-1",
  organization_id: "org-1",
  harness_id: "harness-1",
  agent_id: "agent-1",
  owner_principal_id: "principal_1",
  title: "Session with GPT-4o",
  tags: [],
  model_id: null,
  status: "idle",
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
  started_at: null,
  finished_at: null,
};

const mockUseAgent = jest.fn();
const mockUseSessions = jest.fn();
const mockUseHarnesses = jest.fn();
const mockUpdate = jest.fn();
const mockArchive = jest.fn();
const mutation = (mutateAsync = jest.fn()) => ({
  mutateAsync,
  isPending: false,
  error: null,
  reset: jest.fn(),
});

jest.mock("@/hooks", () => ({
  useAgent: (...args: unknown[]) => mockUseAgent(...args),
  useSessions: (...args: unknown[]) => mockUseSessions(...args),
  useCreateSession: () => mutation(),
  useCapabilities: () => ({ data: [] }),
  useModels: () => ({ data: [] }),
  useExportAgent: () => mutation(),
  useCopyAgent: () => mutation(),
  useHarnesses: () => mockUseHarnesses(),
  useAgentStats: () => ({ data: undefined, isLoading: false, error: null }),
  useAgentMcpAttachments: () => ({ data: [{ name: "github" }, { name: "linear" }] }),
  useAgentCredentials: () => ({ data: [] }),
  useLatestHealthCheckRun: () => ({ data: { config_changed: false } }),
  useUpdateAgent: () => mutation(mockUpdate),
  useDeleteAgent: () => mutation(mockArchive),
  useDestroyAgent: () => mutation(),
  useAgentNameAvailability: () => ({ isChecking: false, available: null }),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => true }),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (flag: string) => flag === "agent_versions",
}));

async function renderPage() {
  const params = Promise.resolve({ agentId: "agent-1" });
  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <AgentPage params={params} />
      </Suspense>,
    );
    await params;
  });
}

async function save() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: /Save changes/ }));
  });
}

beforeEach(() => {
  jest.clearAllMocks();
  mockSearchParams = new URLSearchParams();
  mockUseAgent.mockReturnValue({ data: mockAgent, isLoading: false });
  mockUseSessions.mockReturnValue({
    data: { data: [mockSession], total: 1 },
    isLoading: false,
  });
  mockUseHarnesses.mockReturnValue({
    data: [{ id: "harness_test", name: "generic", display_name: "Generic" }],
    isLoading: false,
  });
  mockUpdate.mockResolvedValue({});
});

describe("AgentPage layout", () => {
  it("shows one tab row with the session count and no MCP or Credentials tabs", async () => {
    await renderPage();

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Agent",
      "Preview",
      "Integrations",
      "Stats",
      "Sessions3",
    ]);
  });

  it("makes the system prompt the page and lists secondary settings with their values", async () => {
    await renderPage();

    expect(screen.getByRole("heading", { name: "Test Agent" })).toBeInTheDocument();
    expect(screen.getByTestId("markdown")).toHaveTextContent("You are helpful");
    expect(screen.getByText("3 words · ~4 tokens")).toBeInTheDocument();
    const more = within(screen.getByRole("navigation", { name: "More settings" }));
    expect(more.getByRole("button", { name: /MCP servers\s*2 attached/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Credentials\s*None/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Network access\s*Inherited/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Health check\s*Not run/ })).toBeInTheDocument();
    // Status badge in view mode, New session is the primary action.
    expect(screen.getByText("active")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /New session/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Save changes/ })).not.toBeInTheDocument();
  });

  it("toggles the prompt between rendered markdown and source", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Source" }));
    expect(screen.queryByTestId("markdown")).not.toBeInTheDocument();
    expect(screen.getByText("You are helpful").tagName).toBe("PRE");
  });

  it("opens the credentials sheet from the old ?tab=credentials deep link", async () => {
    mockSearchParams = new URLSearchParams("tab=credentials");
    await renderPage();

    // The open sheet is modal, so the page behind it is hidden from the a11y tree.
    expect(screen.getByRole("tab", { name: "Agent", hidden: true })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(await screen.findByText("agent credentials")).toBeInTheDocument();
  });

  it("opens a More row in a side sheet", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /MCP servers/ }));
    expect(await screen.findByText("agent mcp")).toBeInTheDocument();
    expect(screen.getByText("Changes here save immediately.")).toBeInTheDocument();
  });

  it("lists every session on the Sessions tab", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    expect(screen.getByText("Session with GPT-4o")).toBeInTheDocument();
    expect(mockUseSessions).toHaveBeenCalledWith("agent-1", { offset: 0, limit: 20 });
  });

  it("keeps an archived agent read-only", async () => {
    mockUseAgent.mockReturnValue({ data: { ...mockAgent, status: "archived" }, isLoading: false });
    await renderPage();

    expect(screen.queryByRole("button", { name: /^Edit/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit prompt" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Default model")).not.toBeInTheDocument();
    expect(screen.getByText("Organization default")).toBeInTheDocument();
  });
});

describe("AgentPage edit mode", () => {
  it("edits in place: same layout, Save and Discard replace New session", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Edit prompt" }));

    expect(screen.getByText("Editing")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Changes apply to new sessions only. Running sessions keep the current definition.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "System prompt" })).toHaveValue("You are helpful");
    expect(screen.getByRole("button", { name: /Discard/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /New session/ })).not.toBeInTheDocument();
    expect(screen.getByTestId("capability-selector")).toBeInTheDocument();
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
  });

  it("starts in edit mode from the old /edit redirect", async () => {
    mockSearchParams = new URLSearchParams("mode=edit");
    await renderPage();

    expect(screen.getByRole("textbox", { name: "System prompt" })).toBeInTheDocument();
  });

  it("enters edit mode when a config control changes and saves one request", async () => {
    await renderPage();

    const tags = screen.getByLabelText("Tags");
    fireEvent.change(tags, { target: { value: "support" } });
    fireEvent.keyDown(tags, { key: "Enter" });
    expect(screen.getByText("Editing")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("textbox", { name: "System prompt" }), {
      target: { value: "Draft prompt" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Select memory" }));
    await save();

    expect(mockUpdate).toHaveBeenCalledTimes(1);
    const request = mockUpdate.mock.calls[0][0].request;
    expect(request.tags).toEqual(["support"]);
    expect(request.system_prompt).toBe("Draft prompt");
    expect(request.capabilities).toEqual([{ ref: "memory", config: {} }]);
    // Untouched layers are omitted so the server leaves them as they are.
    expect(request).not.toHaveProperty("network_access");
    expect(request).not.toHaveProperty("harness_id");
    expect(request).not.toHaveProperty("initial_files");
    expect(screen.queryByText("Editing")).not.toBeInTheDocument();
  });

  it("feeds unsaved prompt text to the checks and applies their fixes", async () => {
    await renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Edit prompt" }));

    fireEvent.change(screen.getByRole("textbox", { name: "System prompt" }), {
      target: { value: "Draft prompt" },
    });
    expect(screen.getByTestId("checks-prompt")).toHaveTextContent("Draft prompt");
    fireEvent.click(screen.getByRole("button", { name: "Apply check fix" }));
    expect(screen.getByRole("textbox", { name: "System prompt" })).toHaveValue("Fixed prompt");
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it("discards the draft and returns to view mode", async () => {
    await renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Edit prompt" }));
    fireEvent.change(screen.getByRole("textbox", { name: "System prompt" }), {
      target: { value: "Throwaway" },
    });

    fireEvent.click(screen.getByRole("button", { name: /Discard/ }));

    expect(screen.getByTestId("markdown")).toHaveTextContent("You are helpful");
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it("preserves an inherited harness when saving an unrelated edit", async () => {
    mockUseAgent.mockReturnValue({
      data: {
        ...mockAgent,
        effective_harness: {
          id: "harness_effective",
          name: "base",
          display_name: "Base",
          source: "organization_default",
          status: "active",
        },
      },
      isLoading: false,
    });
    mockUseHarnesses.mockReturnValue({
      data: [
        { id: "harness_test", name: "generic", display_name: "Generic" },
        { id: "harness_effective", name: "base", display_name: "Base" },
      ],
      isLoading: false,
    });
    await renderPage();

    expect(screen.getByText("Base")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Branding/ }));
    fireEvent.change(await screen.findByLabelText("Description"), {
      target: { value: "Updated description" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    expect(mockUpdate).toHaveBeenCalledTimes(1);
    expect(mockUpdate.mock.calls[0][0].request.description).toBe("Updated description");
    expect(mockUpdate.mock.calls[0][0].request).not.toHaveProperty("harness_id");
  });

  it("includes network access edited in its sheet", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /Network access/ }));
    fireEvent.change(await screen.findByLabelText("Allowed hosts"), {
      target: { value: "api.example.com\n*.github.com" },
    });
    fireEvent.change(screen.getByLabelText("Blocked hosts"), {
      target: { value: "internal.corp" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    expect(mockUpdate.mock.calls[0][0].request.network_access).toEqual({
      allowed: ["api.example.com", "*.github.com"],
      blocked: ["internal.corp"],
    });
  });

  it("sends an empty network_access object when an existing list is cleared", async () => {
    mockUseAgent.mockReturnValue({
      data: { ...mockAgent, network_access: { allowed: ["api.example.com"] } },
      isLoading: false,
    });
    await renderPage();

    expect(screen.getByRole("button", { name: /Network access\s*1 allowed/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Network access/ }));
    fireEvent.change(await screen.findByLabelText("Allowed hosts"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    // {} clears the layer server-side (omitting would leave it unchanged).
    expect(mockUpdate.mock.calls[0][0].request.network_access).toEqual({});
  });

  it("opens Branding when the name fails validation instead of saving", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /Branding/ }));
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Bad Name" } });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    expect(mockUpdate).not.toHaveBeenCalled();
    expect(await screen.findByLabelText("Name")).toHaveAttribute("aria-invalid", "true");
  });
});
