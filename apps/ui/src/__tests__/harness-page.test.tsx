import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { Suspense } from "react";
import HarnessPage from "@/app/(main)/harnesses/[harnessId]/page";
import type { Harness } from "@/lib/api/types";

let mockSearchParams = new URLSearchParams();
const push = jest.fn();
const replace = jest.fn();

jest.mock("next/navigation", () => ({
  usePathname: () => "/harnesses/harness-1",
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

jest.mock("@/components/harnesses/harness-preview", () => ({
  HarnessPreview: ({ systemPrompt }: { systemPrompt?: string }) => (
    <div data-testid="harness-preview">{systemPrompt}</div>
  ),
}));
jest.mock("@/components/integration/integration-guide", () => ({
  IntegrationGuide: () => <div>harness integrate</div>,
}));
jest.mock("@/components/stats/resource-stats-panel", () => ({
  ResourceStatsPanel: () => <div>harness stats</div>,
}));
jest.mock("@/components/initial-files-editor", () => ({
  InitialFilesEditor: () => <div data-testid="initial-files-editor" />,
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
jest.mock("@/components/harness/harness-select", () => ({
  HarnessSelect: ({
    id,
    value,
    onValueChange,
  }: {
    id?: string;
    value: string;
    onValueChange: (value: string) => void;
  }) => <input id={id} value={value} onChange={(event) => onValueChange(event.target.value)} />,
}));

const mockHarness: Harness = {
  id: "harness-1",
  name: "test-harness",
  display_name: "Test Harness",
  description: "A test harness",
  system_prompt: "You are a harness",
  parent_harness_id: null,
  default_model_id: null,
  tags: [],
  capabilities: [],
  initial_files: [],
  is_built_in: false,
  status: "active",
  session_count: 2,
  app_count: 1,
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

const mockUseHarness = jest.fn();
const mockUpdate = jest.fn();
const mutation = (mutateAsync = jest.fn()) => ({
  mutateAsync,
  isPending: false,
  error: null,
  reset: jest.fn(),
});

jest.mock("@/hooks", () => ({
  useHarness: (...args: unknown[]) => mockUseHarness(...args),
  useHarnesses: () => ({ data: [] }),
  useCapabilities: () => ({ data: [] }),
  useModels: () => ({ data: [] }),
  useHarnessStats: () => ({ data: undefined, isLoading: false, error: null }),
  useUpdateHarness: () => mutation(mockUpdate),
  useDeleteHarness: () => mutation(),
  useDestroyHarness: () => mutation(),
  useCopyHarness: () => mutation(),
  useHarnessNameAvailability: () => ({ isChecking: false, available: null }),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => true }),
}));

async function renderPage() {
  const params = Promise.resolve({ harnessId: "harness-1" });
  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <HarnessPage params={params} />
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
  mockUseHarness.mockReturnValue({ data: mockHarness, isLoading: false });
  mockUpdate.mockResolvedValue({});
});

describe("HarnessPage layout", () => {
  it("shows one tab row and makes the system prompt the page", async () => {
    await renderPage();

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Harness",
      "Preview",
      "Integrate",
      "Stats",
    ]);
    expect(screen.getByRole("heading", { name: "Test Harness" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Capabilities" })).toBeInTheDocument();
    expect(screen.getByText("No capabilities enabled.")).toBeInTheDocument();
    expect(screen.queryByTestId("markdown")).not.toBeInTheDocument();
    const more = within(screen.getByRole("navigation", { name: "More settings" }));
    expect(more.getByRole("button", { name: /System prompt\s*4 words/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Branding\s*Description/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Starter files\s*None/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Network access\s*Unrestricted/ })).toBeInTheDocument();
    expect(more.getByRole("button", { name: /Usage\s*2 sessions · 1 app/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Save changes/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Archive/ })).not.toBeInTheDocument();
  });

  it("says when a harness contributes no base prompt", async () => {
    mockUseHarness.mockReturnValue({
      data: { ...mockHarness, system_prompt: null },
      isLoading: false,
    });
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /System prompt/ }));
    expect(await screen.findByText(/contributes no base prompt/)).toBeInTheDocument();
  });

  it("keeps a built-in harness read-only, including the old edit link", async () => {
    mockSearchParams = new URLSearchParams("mode=edit");
    mockUseHarness.mockReturnValue({
      data: { ...mockHarness, is_built_in: true },
      isLoading: false,
    });
    await renderPage();

    expect(screen.getByText("Built-in")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Edit/ })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Default model")).not.toBeInTheDocument();
    expect(screen.getByText("Organization default")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Save changes/ })).not.toBeInTheDocument();
  });

  it("opens the old overview tab on the harness workspace", async () => {
    mockSearchParams = new URLSearchParams("tab=overview");
    await renderPage();

    expect(screen.getByRole("tab", { name: "Harness" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("region", { name: "Capabilities" })).toBeInTheDocument();
  });

  it("shows integrate and stats on their tabs", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("tab", { name: "Integrate" }));
    expect(screen.getByText("harness integrate")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Stats" }));
    expect(screen.getByText("harness stats")).toBeInTheDocument();
  });
});

describe("HarnessPage edit mode", () => {
  it("edits in place: same layout, Save and Discard replace Edit", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Edit capabilities" }));

    expect(screen.getByText("Editing")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Changes apply to new sessions only. Running sessions keep the current definition.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("capability-selector")).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Instructions" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Discard/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
  });

  it("starts in edit mode from the old /edit redirect", async () => {
    mockSearchParams = new URLSearchParams("mode=edit");
    await renderPage();

    expect(screen.getByTestId("capability-selector")).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Instructions" })).not.toBeInTheDocument();
  });

  it("enters edit mode when a config control changes and omits untouched layers", async () => {
    await renderPage();

    const tags = screen.getByLabelText("Tags");
    fireEvent.change(tags, { target: { value: "research" } });
    fireEvent.keyDown(tags, { key: "Enter" });
    expect(screen.getByText("Editing")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Select memory" }));
    await save();

    expect(mockUpdate).toHaveBeenCalledTimes(1);
    const request = mockUpdate.mock.calls[0][0].request;
    expect(request.tags).toEqual(["research"]);
    expect(request.capabilities).toEqual([{ ref: "memory", config: {} }]);
    expect(request).not.toHaveProperty("system_prompt");
    expect(request).not.toHaveProperty("parent_harness_id");
    expect(request).not.toHaveProperty("network_access");
    expect(request).not.toHaveProperty("initial_files");
    expect(screen.queryByText("Editing")).not.toBeInTheDocument();
  });

  it("sends a cleared prompt and a changed parent", async () => {
    await renderPage();

    fireEvent.click(screen.getByRole("button", { name: /System prompt/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit prompt" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Instructions" }), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByLabelText("Parent harness"), {
      target: { value: "harness-base" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await save();

    expect(mockUpdate.mock.calls[0][0].request.system_prompt).toBe("");
    expect(mockUpdate.mock.calls[0][0].request.parent_harness_id).toBe("harness-base");
  });

  it("discards the draft and returns to view mode", async () => {
    await renderPage();
    fireEvent.click(screen.getByRole("button", { name: /System prompt/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit prompt" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Instructions" }), {
      target: { value: "Throwaway" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));

    fireEvent.click(screen.getByRole("button", { name: /Discard/ }));

    expect(screen.getByRole("button", { name: /System prompt\s*4 words/ })).toBeInTheDocument();
    expect(mockUpdate).not.toHaveBeenCalled();
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
