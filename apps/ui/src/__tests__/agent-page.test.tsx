import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { Suspense } from "react";
import AgentPage from "@/app/(main)/agents/[agentId]/page";
import type { Agent, Session } from "@/lib/api/types";

let mockSearchParams = new URLSearchParams();
const push = jest.fn();
const replace = jest.fn();

jest.mock("@/hooks/use-virtual-users", () => ({
  useVirtualUsers: () => ({ data: [] }),
  useVirtualUser: () => ({ data: undefined }),
}));

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
jest.mock("@/components/agents/sandbox-policy-editor", () => ({
  SandboxPolicyEditor: ({ onChange }: { onChange: (value: Record<string, unknown>) => void }) => (
    <button
      type="button"
      onClick={() =>
        onChange({
          default: "build",
          templates: {
            build: {
              target: { kind: "managed", provider: "daytona" },
              durability: "checkpointed",
            },
          },
        })
      }
    >
      Add Daytona sandbox
    </button>
  ),
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
const mockUseAgentChannels = jest.fn();
const mockUseAgentTriggers = jest.fn();

jest.mock("@/hooks/use-agent-channels", () => ({
  useAgentChannels: () => mockUseAgentChannels(),
}));
jest.mock("@/hooks/use-agent-triggers", () => ({
  useAgentTriggers: () => mockUseAgentTriggers(),
}));

const mockUseSessions = jest.fn();
const mockUseHarnesses = jest.fn();
const mockUpdate = jest.fn();
const mockArchive = jest.fn();
const mockCreateSession = jest.fn();
const mutation = (mutateAsync = jest.fn()) => ({
  mutateAsync,
  isPending: false,
  error: null,
  reset: jest.fn(),
});

jest.mock("@/hooks", () => ({
  useAgent: (...args: unknown[]) => mockUseAgent(...args),
  useSessions: (...args: unknown[]) => mockUseSessions(...args),
  useCreateSession: () => mutation(mockCreateSession),
  useCapabilities: () => ({ data: [] }),
  useModels: () => ({ data: [] }),
  useExportAgent: () => mutation(),
  useCopyAgent: () => mutation(),
  useHarnesses: () => mockUseHarnesses(),
  useAgentStats: () => ({ data: undefined, isLoading: false, error: null }),
  useAgentMcpAttachments: () => ({
    data: [{ name: "github" }, { name: "linear" }],
  }),
  useAgentCredentials: () => ({ data: [] }),
  useLatestHealthCheckRun: () => ({ data: { config_changed: false } }),
  useUpdateAgent: () => mutation(mockUpdate),
  useDeleteAgent: () => mutation(mockArchive),
  useDestroyAgent: () => mutation(),
  useAgentNameAvailability: () => ({ isChecking: false, available: null }),
  useAvatarPresets: () => ({
    catalog: { data: [], isLoading: false },
    current: {},
  }),
  useAgentAvatar: () => ({
    selectPreset: {
      isPending: false,
      error: null,
      reset: jest.fn(),
      mutateAsync: jest.fn(),
    },
    upload: { mutate: jest.fn(), isPending: false, error: null },
    remove: { mutate: jest.fn(), isPending: false, error: null },
  }),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => true }),
}));

let mockManagerNotes = { content: "", revision: 0 };
jest.mock("@/hooks/use-change-history", () => ({
  useEntityHistory: () => ({
    data: { pages: [{ entries: [] }] },
    isLoading: false,
    error: null,
    hasNextPage: false,
  }),
  useEntityRevision: () => ({ data: undefined, isLoading: false, error: null }),
  useEntityRevisionDiff: () => ({ data: [], isLoading: false, error: null }),
  useRestoreEntityRevision: () => ({
    mutateAsync: jest.fn(),
    isPending: false,
    reset: jest.fn(),
  }),
  useManagerContext: () => ({
    data: mockManagerNotes,
    isLoading: false,
    error: null,
  }),
  useSetManagerContext: () => ({
    mutateAsync: jest.fn(),
    isPending: false,
    reset: jest.fn(),
  }),
}));

jest.mock("@/hooks/use-members", () => ({ useMembers: () => ({ data: [] }) }));

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
  mockUseAgentChannels.mockReturnValue({ data: [] });
  mockUseAgentTriggers.mockReturnValue({ data: [] });
  mockUseSessions.mockReturnValue({
    data: { data: [mockSession], total: 1 },
    isLoading: false,
  });
  mockUseHarnesses.mockReturnValue({
    data: [{ id: "harness_test", name: "generic", display_name: "Generic" }],
    isLoading: false,
  });
  mockUpdate.mockResolvedValue({});
  mockCreateSession.mockResolvedValue({ id: "session-chat-1" });
  mockManagerNotes = { content: "", revision: 0 };
});

describe("AgentPage layout", () => {
  it("shows one tab row with the session count and no MCP or Credentials tabs", async () => {
    await renderPage();

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Agent",
      "Preview",
      "Integrations0",
      "Stats",
      "Sessions3",
    ]);
  });

  it("counts channels and native triggers while counting legacy schedules once", async () => {
    mockUseAgentChannels.mockReturnValue({
      data: [
        { channel_type: "ag_ui", enabled: true },
        { channel_type: "slack", enabled: false },
        { channel_type: "schedule", enabled: false },
      ],
    });
    mockUseAgentTriggers.mockReturnValue({
      data: [{ enabled: true }, { enabled: false }],
    });
    await renderPage();

    expect(screen.getByRole("tab", { name: "Integrations 5" })).toBeInTheDocument();
  });

  it.each(["channels", "triggers"])("hides the total until %s have loaded", async (pending) => {
    mockUseAgentChannels.mockReturnValue({
      data: pending === "channels" ? undefined : [{}],
    });
    mockUseAgentTriggers.mockReturnValue({
      data: pending === "triggers" ? undefined : [{}],
    });
    await renderPage();

    expect(screen.getByRole("tab", { name: "Integrations" })).toBeInTheDocument();
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
    expect(more.getByRole("button", { name: /Primary sandbox\s*None/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Health check\s*Not run/ })).toBeInTheDocument();
    // Status badge in view mode, Test in Playground is the primary action.
    expect(screen.getByText("active")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Test in Playground/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Save changes/ })).not.toBeInTheDocument();
  });

  it("shows the fixed Bashkit sandbox inherited from the Harness", async () => {
    mockUseAgent.mockReturnValue({
      data: { ...mockAgent, harness_id: "harness_bashkit" },
      isLoading: false,
    });
    mockUseHarnesses.mockReturnValue({
      data: [
        {
          id: "harness_bashkit",
          name: "bashkit-worker",
          display_name: "Bashkit Worker",
        },
      ],
      isLoading: false,
    });
    await renderPage();

    expect(
      screen.getByRole("button", {
        name: /Primary sandbox\s*Bashkit Virtual Workspace/,
      }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Primary sandbox/ }));
    expect(
      screen.getByText("Locked by the selected Harness.", { exact: false }),
    ).toBeInTheDocument();
  });

  it("opens Playground setup with the Agent preselected without creating a session", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /Test in Playground/ }));

    expect(mockCreateSession).not.toHaveBeenCalled();
    expect(push).toHaveBeenCalledWith("/playground/new?agent=agent-1");
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
    expect(mockUseSessions).toHaveBeenCalledWith("agent-1", {
      offset: 0,
      limit: 20,
    });
  });

  it("writes the selected tab into the URL so a refresh keeps it", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("tab", { name: "Integrations 0" }));

    expect(screen.getByText("agent integrations")).toBeInTheDocument();
    expect(replace).toHaveBeenCalledWith("/agents/agent-1?tab=integrations", {
      scroll: false,
    });
  });

  it("restores the tab from the URL on load", async () => {
    mockSearchParams = new URLSearchParams("tab=stats");
    await renderPage();

    expect(screen.getByText("agent stats")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Stats" })).toHaveAttribute("aria-selected", "true");
  });

  it("returns to the bare agent path from another tab and keeps other query params", async () => {
    mockSearchParams = new URLSearchParams("tab=preview&mode=edit");
    await renderPage();

    fireEvent.click(screen.getByRole("tab", { name: "Agent" }));

    expect(replace).toHaveBeenCalledWith("/agents/agent-1?mode=edit", {
      scroll: false,
    });
  });

  it("keeps the open tab when leaving edit mode", async () => {
    mockSearchParams = new URLSearchParams("mode=edit&tab=integrations");
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /Discard/ }));

    expect(replace).toHaveBeenCalledWith("/agents/agent-1?tab=integrations", {
      scroll: false,
    });
  });

  it("keeps an archived agent read-only", async () => {
    mockUseAgent.mockReturnValue({
      data: { ...mockAgent, status: "archived" },
      isLoading: false,
    });
    await renderPage();

    expect(screen.queryByRole("button", { name: /^Edit/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit prompt" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Default model")).not.toBeInTheDocument();
    expect(screen.getByText("Organization default")).toBeInTheDocument();
  });

  it("keeps a built-in agent read-only and offers Copy but not Archive", async () => {
    mockUseAgent.mockReturnValue({
      data: { ...mockAgent, is_built_in: true },
      isLoading: false,
    });
    mockSearchParams = new URLSearchParams("mode=edit");
    await renderPage();

    expect(screen.getByText("Built-in")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Edit/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Save changes/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit prompt" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "More actions for Test Agent" }));
    const items = (await screen.findAllByRole("menuitem")).map((item) => item.textContent);
    expect(items).toContain("Copy");
    expect(items).not.toContain("Archive agent");
    expect(items).not.toContain("Delete agent");
  });
});

describe("AgentPage edit mode", () => {
  it("edits in place: same layout, Save and Discard replace Test in Playground", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Edit prompt" }));

    expect(screen.getByText("Editing")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Changes apply to new sessions only. Running sessions keep the current definition.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Instructions" })).toHaveValue("You are helpful");
    expect(screen.getByRole("button", { name: /Discard/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Test in Playground/ })).not.toBeInTheDocument();
    expect(screen.getByTestId("capability-selector")).toBeInTheDocument();
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
  });

  it("starts in edit mode from the old /edit redirect", async () => {
    mockSearchParams = new URLSearchParams("mode=edit");
    await renderPage();

    expect(screen.getByRole("textbox", { name: "Instructions" })).toBeInTheDocument();
  });

  it("enters edit mode when a config control changes and saves one request", async () => {
    await renderPage();

    const tags = screen.getByLabelText("Tags");
    fireEvent.change(tags, { target: { value: "support" } });
    fireEvent.keyDown(tags, { key: "Enter" });
    expect(screen.getByText("Editing")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("textbox", { name: "Instructions" }), {
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

    fireEvent.change(screen.getByRole("textbox", { name: "Instructions" }), {
      target: { value: "Draft prompt" },
    });
    expect(screen.getByTestId("checks-prompt")).toHaveTextContent("Draft prompt");
    fireEvent.click(screen.getByRole("button", { name: "Apply check fix" }));
    expect(screen.getByRole("textbox", { name: "Instructions" })).toHaveValue("Fixed prompt");
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it("discards the draft and returns to view mode", async () => {
    await renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Edit prompt" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Instructions" }), {
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

  it("includes sandbox policy edited in their sheet", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /Primary sandbox/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Add Daytona sandbox" }));
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    expect(mockUpdate).toHaveBeenCalledTimes(1);
    expect(mockUpdate.mock.calls[0][0].request.sandbox_policy).toEqual({
      default: "build",
      templates: {
        build: {
          target: { kind: "managed", provider: "daytona" },
          durability: "checkpointed",
        },
      },
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
    fireEvent.change(await screen.findByLabelText("Allowed hosts"), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    // {} clears the layer server-side (omitting would leave it unchanged).
    expect(mockUpdate.mock.calls[0][0].request.network_access).toEqual({});
  });

  it("opens Branding when the name fails validation instead of saving", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /Branding/ }));
    fireEvent.change(await screen.findByLabelText("Name"), {
      target: { value: "Bad Name" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    expect(mockUpdate).not.toHaveBeenCalled();
    expect(await screen.findByLabelText("Name")).toHaveAttribute("aria-invalid", "true");
  });
});

describe("AgentPage entity actions", () => {
  it("puts History and Manager notes in the menu, not Version history", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: "More actions for Test Agent" }));
    const items = (await screen.findAllByRole("menuitem")).map((item) => item.textContent);
    expect(items).toEqual([
      "Copy",
      "Export package (ZIP)",
      "Export Markdown",
      "History",
      "Manager notes",
      "Archive agent",
    ]);
    fireEvent.click(screen.getByRole("menuitem", { name: "Manager notes" }));
    expect(replace).toHaveBeenCalledWith("/agents/agent-1?sheet=notes", {
      scroll: false,
    });
  });

  it("opens the History sheet from ?sheet=history", async () => {
    mockSearchParams = new URLSearchParams("sheet=history");
    await renderPage();

    expect(await screen.findByText("No changes recorded yet.")).toBeInTheDocument();
  });

  it("sends retired ?tab=versions links to the History sheet", async () => {
    mockSearchParams = new URLSearchParams("tab=versions");
    await renderPage();

    expect(replace).toHaveBeenCalledWith("/agents/agent-1?sheet=history", {
      scroll: false,
    });
    expect(screen.queryByText("agent versions")).not.toBeInTheDocument();
  });

  it("sends the optional reason with Save and never requires one", async () => {
    mockSearchParams = new URLSearchParams("mode=edit");
    await renderPage();

    fireEvent.change(screen.getByLabelText(/Reason for this change/), {
      target: { value: "shorter answers" },
    });
    await save();
    expect(mockUpdate.mock.calls[0][0].reason).toBe("shorter answers");
  });

  it("sends the reason with Archive", async () => {
    mockArchive.mockResolvedValue(undefined);
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: "More actions for Test Agent" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Archive agent" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/Reason for this change/), {
      target: { value: "replaced by v2" },
    });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Archive agent" }));
    });
    expect(mockArchive).toHaveBeenCalledWith({
      id: "agent-1",
      reason: "replaced by v2",
    });
  });

  it("shows the manager notes hint only in edit mode", async () => {
    mockManagerNotes = { content: "Keep it kid friendly.", revision: 2 };
    await renderPage();
    expect(screen.queryByText("This agent has manager notes")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Edit prompt" }));
    expect(screen.getByText("This agent has manager notes")).toBeInTheDocument();
  });
});
