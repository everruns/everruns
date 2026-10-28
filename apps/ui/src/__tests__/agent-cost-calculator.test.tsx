import { fireEvent, render, screen } from "@testing-library/react";
import { AgentCostCalculator } from "@/components/agents/agent-cost-estimate";
import { LocaleProvider } from "@/providers/locale-provider";
import type { ModelWithProvider } from "@/lib/api/types";

const model = {
  id: "model-1",
  display_name: "Test model",
  provider_name: "Test provider",
  profile: { cost: { input: 2, output: 10 } },
} as ModelWithProvider;
let mockModels: unknown;
let mockDefault: unknown;
let mockPreview: unknown;
const mockRefetch = jest.fn();
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => mockModels,
  useDefaultModel: () => mockDefault,
}));
jest.mock("@/hooks/use-harnesses", () => ({ useHarnesses: () => ({ data: [] }) }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org-1" } }),
}));
jest.mock("@tanstack/react-query", () => ({ useQuery: () => mockPreview }));

const props = {
  modelId: "model-1",
  maxOutputTokens: "",
  previewRequest: { system_prompt: "draft", tools: [] },
};
beforeEach(() => {
  localStorage.clear();
  mockModels = { data: [model] };
  mockDefault = { data: model };
  mockPreview = { data: { system_prompt: "abcd", tools: [] }, refetch: mockRefetch };
  mockRefetch.mockClear();
});

it.each([
  ["uk", "Вартість на місяць"],
  ["fr", "Coût mensuel"],
  ["de", "Monatliche Kosten"],
  ["es", "Coste mensual"],
  ["zh-CN", "每月费用"],
])("localizes the calculator and formats USD amounts for %s", (locale, monthlyLabel) => {
  localStorage.setItem("everruns.ui-language", locale);
  render(
    <LocaleProvider>
      <AgentCostCalculator {...props} />
    </LocaleProvider>,
  );
  expect(screen.getByText(monthlyLabel)).toBeInTheDocument();
  expect(
    screen.getByText(
      new Intl.NumberFormat(locale, { style: "currency", currency: "USD" }).format(0.09701),
      { normalizer: (value) => value },
    ),
  ).toBeInTheDocument();
  expect(screen.queryByText("Advanced assumptions")).not.toBeInTheDocument();
});

it("updates task costs and monthly volume independently", () => {
  render(<AgentCostCalculator {...props} />);
  expect(screen.getByText("$0.10")).toBeInTheDocument();
  expect(screen.getByText("$29.10")).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "20" } });
  expect(screen.getByText("$58.21")).toBeInTheDocument();
  expect(screen.getByText("$0.10")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /Quick answer/ }));
  expect(screen.getByLabelText("Model calls per task")).not.toBeVisible();
  fireEvent.click(screen.getByText("Advanced assumptions", { exact: true }));
  expect(screen.getByLabelText("Model calls per task")).toBeVisible();
  expect(screen.getByLabelText("Model calls per task")).toHaveValue(1);
  expect(screen.getByText("<$0.01")).toBeInTheDocument();
});

it("withholds invalid numeric estimates and allows zero monthly volume", () => {
  render(<AgentCostCalculator {...props} />);
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "0" } });
  expect(screen.getByText("0 tasks per month")).toBeInTheDocument();
  fireEvent.click(screen.getByText("Advanced assumptions", { exact: true }));
  fireEvent.change(screen.getByLabelText("Model calls per task"), { target: { value: "" } });
  expect(screen.getByText(/Check the numbers under Advanced assumptions/)).toBeInTheDocument();
  expect(screen.queryByText("/ task")).not.toBeInTheDocument();
  expect(screen.getByLabelText("Model calls per task")).toHaveAttribute("aria-invalid", "true");
});

it("uses the organization default only when there is no explicit model", () => {
  const { rerender } = render(<AgentCostCalculator {...props} modelId="" />);
  expect(screen.getByText(/Organization default/)).toBeInTheDocument();
  rerender(<AgentCostCalculator {...props} modelId="missing-model" />);
  expect(screen.getByText(/Choose a model for this agent/)).toBeInTheDocument();
  expect(screen.queryByText("/ task")).not.toBeInTheDocument();
});

it("does not present missing pricing as free", () => {
  mockModels = { data: [{ ...model, profile: undefined }] };
  render(<AgentCostCalculator {...props} />);
  expect(screen.getByText(/We don’t have prices/)).toBeInTheDocument();
  expect(screen.queryByText("$0.00")).not.toBeInTheDocument();
});

it("shows loading, failure and retry without stale figures", () => {
  mockPreview = { isPending: true };
  const { rerender } = render(<AgentCostCalculator {...props} />);
  expect(screen.getByText(/Calculating your estimate/)).toBeInTheDocument();
  mockPreview = { isError: true, data: { system_prompt: "abcd", tools: [] }, refetch: mockRefetch };
  mockModels = { data: [model], refetch: jest.fn() };
  rerender(<AgentCostCalculator {...props} />);
  expect(screen.getByRole("alert")).toHaveTextContent(/We couldn’t load/);
  expect(screen.queryByText("/ task")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(mockRefetch).toHaveBeenCalledTimes(1);
});

it("explains an impossible workload instead of quoting it", () => {
  mockModels = {
    data: [{ ...model, profile: { ...model.profile, limits: { context: 1000, output: 500 } } }],
  };
  render(<AgentCostCalculator {...props} />);
  expect(screen.getByText(/This task is too large/)).toBeInTheDocument();
  expect(screen.queryByText("/ task")).not.toBeInTheDocument();
});
