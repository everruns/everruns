import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentCostPlanner } from "@/components/agents/agent-cost-planner";
import { planAgentCosts, previewAgent } from "@/lib/api/agents";
const model = { id: "model-1", display_name: "Model", profile: { cost: { input: 2, output: 10 } } };
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
function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AgentCostPlanner {...props} />
    </QueryClientProvider>,
  );
}
it("infers once, calculates sliders locally, and regenerates only on request", async () => {
  setup();
  await screen.findByText("Typical tasks for your agent");
  expect(planMock).toHaveBeenCalledTimes(1);
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "200" } });
  fireEvent.change(screen.getAllByLabelText("Frequency")[0], { target: { value: "75" } });
  expect(planMock).toHaveBeenCalledTimes(1);
  fireEvent.change(screen.getByLabelText("What should we plan for? (optional)"), {
    target: { value: "Short reports only" },
  });
  expect(planMock).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByText("Update estimate"));
  await waitFor(() => expect(planMock).toHaveBeenCalledTimes(2));
  expect(planMock.mock.calls[1][1]).toBe("Short reports only");
});
it("offers the manual calculator after inference fails", async () => {
  planMock.mockRejectedValue(new Error("unavailable"));
  setup();
  await screen.findByRole("alert");
  fireEvent.click(screen.getByRole("button", { name: "Manual calculator" }));
  expect(screen.getByText("Quick answer")).toBeInTheDocument();
});
it("does not reuse suggestions across organizations", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <AgentCostPlanner {...props} />
    </QueryClientProvider>,
  );
  await screen.findByText("Typical tasks for your agent");
  org = "org-b";
  view.rerender(
    <QueryClientProvider client={client}>
      <AgentCostPlanner {...props} />
    </QueryClientProvider>,
  );
  await waitFor(() => expect(planMock).toHaveBeenCalledTimes(2));
});
