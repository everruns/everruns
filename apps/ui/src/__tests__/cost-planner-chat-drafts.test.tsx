import { act, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  PageViewRegistryContext,
  usePageViewRegistry,
} from "@/components/chat-mode/page-view-state";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import { AgentCostEstimate } from "@/components/agents/agent-cost-estimate";
import { AgentCostPlanner } from "@/components/agents/agent-cost-planner";
import { planAgentCosts, previewAgent } from "@/lib/api/agents";
const model = { id: "model-1", display_name: "Model", profile: { cost: { input: 2, output: 10 } } };
jest.mock("@/hooks/use-agent-cost-pricing", () => ({
  useAgentCostPricing: () => ({
    status: "ready",
    modelBilling: "external",
    rates: [
      {
        id: "storage",
        label: "Stored content",
        unit: "GiB",
        usdPerUnit: 0.05,
        billing: "credits",
        period: "month",
      },
    ],
  }),
}));
let org = "org-a";
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({ data: [model] }),
  useDefaultModel: () => ({ data: model }),
}));
jest.mock("@/hooks/use-harnesses", () => ({ useHarnesses: () => ({ data: [] }) }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: org } }),
}));
jest.mock("@/lib/api/agents", () => ({ planAgentCosts: jest.fn(), previewAgent: jest.fn() }));
const task = {
  name: "Research companies",
  description: "Find products",
  assumption: "Three searches",
  calls: 4,
  message_tokens: 500,
  output_tokens: 800,
  tool_result_tokens: 1000,
};
const props = {
  modelId: "model-1",
  maxOutputTokens: "",
  previewRequest: { system_prompt: "Research public companies" },
};
const planMock = jest.mocked(planAgentCosts);
beforeEach(() => {
  org = "org-a";
  jest.clearAllMocks();
  planMock.mockResolvedValue({
    tasks: [task, { ...task, name: "Compare" }, { ...task, name: "Report" }],
  });
  jest.mocked(previewAgent).mockResolvedValue({
    system_prompt: "Research",
    tools: [],
    features: [],
    initial_files: [],
    findings: [],
  });
});

const forms = new Map<string, { adapter: PageDraftAdapter; priority: number }>();
const mode: ChatModeState = {
  page: { route: "/agents/agent-1", label: "Agent" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: (page, adapter, priority = 0) => {
    const name = page.form_name!;
    const entry = { adapter: adapter!, priority };
    forms.set(name, entry);
    return () => {
      if (forms.get(name) === entry) forms.delete(name);
    };
  },
};
beforeEach(() => {
  forms.clear();
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});
function setup(content = <AgentCostPlanner {...props} />) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const registry = renderHook(() => usePageViewRegistry(mode.page.route));
  const view = render(
    <QueryClientProvider client={client}>
      <PageChatProvider value={mode}>
        <PageViewRegistryContext.Provider value={registry.result.current.registry}>
          {content}
        </PageViewRegistryContext.Provider>
      </PageChatProvider>
    </QueryClientProvider>,
  );
  return { ...view, pageView: () => registry.result.current.read(mode.page.route) };
}
function draft(name: string) {
  const value = forms.get(name)!;
  expect(value).toBeDefined();
  expect(value.priority).toBeGreaterThan(20);
  return value.adapter;
}
function patch(name: string, changes: object) {
  const adapter = draft(name);
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, changes);
  });
  return { adapter, before };
}
it("edits only local visible planning inputs without requesting a new estimate", async () => {
  setup();
  await screen.findByText("Typical tasks for your agent");
  patch("planner", { guidance: "Short reports" });
  expect(screen.getByLabelText("What should we plan for? (optional)")).toHaveValue("Short reports");
  patch("projection", { volume: "250" });
  expect(screen.getByLabelText("Tasks per day")).toHaveValue(250);
  expect(planMock).toHaveBeenCalledTimes(1);
  expect(
    screen.queryByText("Review these changes, then save to apply them."),
  ).not.toBeInTheDocument();
});
it("expires smart inputs on a manual switch and exposes the manual calculator", async () => {
  const view = setup();
  await screen.findByText("Typical tasks for your agent");
  const old = draft("projection");
  const token = old.describe().binding;
  const planner = draft("planner");
  const before = planner.read(planner.describe().binding);
  expect(() => planner.patch(before.binding, before.revision, { manual: true })).toThrow();
  fireEvent.click(screen.getByRole("button", { name: "Manual calculator" }));
  expect(() => old.read(token)).toThrow();
  expect(() => planner.read(before.binding)).toThrow();
  expect(forms.has("planner")).toBe(false);
  patch("manual_calculator", { calls: "7", tasksPerDay: "30" });
  expect(screen.getByLabelText("Tasks per day")).toHaveValue(30);
  fireEvent.click(screen.getByText("Advanced assumptions"));
  expect(screen.getByLabelText("Model calls per task")).toHaveValue(7);
  expect(planMock).toHaveBeenCalledTimes(1);
  const manual = draft("manual_calculator");
  const binding = manual.describe().binding;
  view.unmount();
  expect(() => manual.read(binding)).toThrow();
});

it("keeps external price edits local and rejects host-price, metadata and row injection", async () => {
  setup();
  await screen.findByText("Typical tasks for your agent");
  fireEvent.click(screen.getByRole("button", { name: "Add cost" }));
  let adapter = draft("operating_costs");
  let before = adapter.read(adapter.describe().binding);
  const costs = before.values.costs as Record<string, Record<string, string>>;
  const id = Object.keys(costs)[0];
  patch("operating_costs", {
    costs: { ...costs, [id]: { ...costs[id], quantity: "2", usdPerUnit: "0.5" } },
  });
  expect(screen.getByLabelText("Quantity per task (minute)")).toHaveValue(2);
  expect(screen.getByLabelText("USD per minute")).toHaveValue(0.5);
  adapter = draft("operating_costs");
  before = adapter.read(adapter.describe().binding);
  expect(() =>
    adapter.patch(before.binding, before.revision, {
      costs: { [id]: { quantity: "2", usdPerUnit: "0.5", billing: "credits" } },
    }),
  ).toThrow();
  expect(() =>
    adapter.patch(before.binding, before.revision, {
      costs: { extra: { quantity: "1", usdPerUnit: "0" } },
    }),
  ).toThrow();
  const old = adapter;
  const token = before.binding;
  patch("operating_costs", { selection: "rate:storage" });
  fireEvent.click(screen.getByRole("button", { name: "Add cost" }));
  expect(() => old.read(token)).toThrow();
  adapter = draft("operating_costs");
  before = adapter.read(adapter.describe().binding);
  const latest = before.values.costs as Record<string, Record<string, string>>;
  const host = Object.keys(latest).find((key) => key !== id)!;
  expect(latest[host]).not.toHaveProperty("usdPerUnit");
  expect(() =>
    adapter.patch(before.binding, before.revision, {
      costs: { ...latest, [host]: { ...latest[host], usdPerUnit: "0" } },
    }),
  ).toThrow();
  patch("operating_costs", { costs: { ...latest, [host]: { quantity: "100" } } });
  expect(screen.getByLabelText("Quantity per month (GiB)")).toHaveValue(100);
  expect(screen.getByLabelText("USD per GiB")).toHaveValue(0.05);
  expect(screen.getByLabelText("USD per GiB")).toHaveAttribute("readonly");
  expect(planMock).toHaveBeenCalledTimes(1);
});

it("registers only inside the open drawer and expires its bindings on close", async () => {
  setup(<AgentCostEstimate {...props} />);
  expect(forms.size).toBe(0);
  fireEvent.click(screen.getByRole("button", { name: "Estimate cost" }));
  await screen.findByText("Typical tasks for your agent");
  const adapter = draft("planner");
  const binding = adapter.describe().binding;
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  await waitFor(() => expect(forms.size).toBe(0));
  expect(() => adapter.read(binding)).toThrow();
});

it("patches each projection assumption independently while preserving other inputs", async () => {
  setup();
  await screen.findByText("Typical tasks for your agent");
  const adapter = draft("projection");
  const initial = adapter.read(adapter.describe().binding);
  expect(Object.keys(initial.patch_schema!.properties as object).sort()).toEqual(
    ["tasks", "mode", "volume", "users", "perUser", "selected", "adjustments"].sort(),
  );
  for (let index = 0; index < 3; index++) {
    for (const field of ["calls", "messageTokens", "outputTokens", "toolResultTokens", "weight"]) {
      const before = adapter.read(adapter.describe().binding);
      const rows = before.values.tasks as Record<string, number>[];
      const tasks = rows.map((row, i) => (i === index ? { ...row, [field]: row[field] + 1 } : row));
      patch("projection", { tasks });
      expect(adapter.read(before.binding).values).toEqual({ ...before.values, tasks });
    }
  }
  for (const [field, value] of Object.entries({
    volume: "250",
    mode: "users",
    users: "60",
    perUser: "3",
    selected: 1,
  })) {
    const before = adapter.read(adapter.describe().binding);
    patch("projection", { [field]: value });
    expect(adapter.read(before.binding).values).toEqual({ ...before.values, [field]: value });
  }
  for (const field of ["cacheReadPercent", "cacheWritePercent", "retryPercent"]) {
    const before = adapter.read(adapter.describe().binding);
    const adjustments = { ...(before.values.adjustments as object), [field]: 10 };
    patch("projection", { adjustments });
    expect(adapter.read(before.binding).values).toEqual({ ...before.values, adjustments });
  }
  const before = adapter.read(adapter.describe().binding);
  expect(() => adapter.patch(before.binding, before.revision, { tasks: [] })).toThrow();
  expect(() =>
    adapter.patch(before.binding, before.revision, { pricing: { modelBilling: "credits" } }),
  ).toThrow();
  expect(planMock).toHaveBeenCalledTimes(1);
});

it("projects the displayed assumptions and totals and clears them with the drawer", async () => {
  const view = setup(<AgentCostEstimate {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Estimate cost" }));
  await screen.findByText("Typical tasks for your agent");
  expect(view.pageView().sections).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        label: "Cost plan assumptions",
        rows: expect.arrayContaining([
          expect.objectContaining({
            name: "Research companies",
            assumption: "Three searches",
            calls: 4,
          }),
        ]),
      }),
      expect.objectContaining({
        label: "Cost projection",
        rows: expect.arrayContaining([expect.objectContaining({ model_billing: "external" })]),
      }),
    ]),
  );
  patch("projection", { volume: "0" });
  expect(view.pageView().sections).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        label: "Cost projection",
        rows: expect.arrayContaining([expect.objectContaining({ monthly_usd: 0 })]),
      }),
    ]),
  );
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  await waitFor(() => expect(view.pageView().sections).toEqual([]));
});
