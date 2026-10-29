import type { ModelProfile } from "@/lib/api/types";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { CostProjectionDashboard } from "@/components/agents/cost-projection-dashboard";
import { LocaleProvider } from "@/providers/locale-provider";
const task = {
  name: "Research a company",
  description: "Summarize products",
  assumption: "Three searches and a report",
  calls: 4,
  message_tokens: 500,
  output_tokens: 800,
  tool_result_tokens: 1000,
};
const props = {
  plan: {
    tasks: [task, { ...task, name: "Answer a question" }, { ...task, name: "Write a brief" }],
  },
  profile: { cost: { input: 2, output: 10 } } as ModelProfile,
  definitionTokens: 100,
};
beforeEach(() => localStorage.clear());
it("includes sandbox costs only after a provider price is supplied", () => {
  render(<CostProjectionDashboard {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Add cost" }));
  expect(screen.getByRole("status")).toHaveTextContent("Prices needed for: Sandbox runtime");
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "0" } });
  expect(screen.getAllByText("$0.00").length).toBeGreaterThan(0);
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "100" } });
  const row = within(screen.getByRole("group", { name: "Sandbox runtime" }));
  fireEvent.change(row.getByLabelText("Quantity per task (minute)"), { target: { value: "5" } });
  fireEvent.change(row.getByLabelText("USD per minute"), { target: { value: "0.01" } });
  expect(screen.getAllByText("$325.20").length).toBeGreaterThan(0);
  fireEvent.click(row.getByRole("button", { name: "Remove" }));
  expect(screen.getAllByText("$175.20").length).toBeGreaterThan(0);
});

it("keeps credit deductions separate from external fixed fees", () => {
  render(
    <CostProjectionDashboard
      {...props}
      pricing={{
        status: "ready",
        creditLabel: "Example credits",
        modelBilling: "credits",
        rates: [
          {
            id: "search",
            label: "Managed search",
            unit: "search",
            usdPerUnit: 0.006,
            billing: "credits",
          },
        ],
      }}
    />,
  );
  fireEvent.change(screen.getByLabelText("Cost category"), { target: { value: "rate:search" } });
  fireEvent.click(screen.getByRole("button", { name: "Add cost" }));
  expect(
    within(screen.getByRole("group", { name: "Managed search" })).getByLabelText("USD per search"),
  ).toHaveAttribute("readonly");
  fireEvent.change(screen.getByLabelText("Cost category"), {
    target: { value: "external:hosting" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add cost" }));
  fireEvent.change(screen.getByLabelText("USD per month"), { target: { value: "10" } });
  expect(screen.getAllByText("$203.20").length).toBeGreaterThan(0);
  expect(screen.getByText("$193.20")).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "0" } });
  expect(screen.getAllByText("$10.00").length).toBeGreaterThan(0);
  expect(screen.getByRole("img")).toBeInTheDocument();
});

it("applies retry allowances and rejects overlapping cache assumptions", () => {
  render(<CostProjectionDashboard {...props} />);
  fireEvent.change(screen.getByLabelText("Extra model spend for retries (%)"), {
    target: { value: "10" },
  });
  expect(screen.getAllByText("$192.72").length).toBeGreaterThan(0);
  fireEvent.change(screen.getByLabelText("Cached input reads (%)"), { target: { value: "80" } });
  fireEvent.change(screen.getByLabelText("Cache writes (%)"), { target: { value: "30" } });
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
});
it("projects tasks or active users without making network requests", () => {
  render(<CostProjectionDashboard {...props} />);
  expect(screen.getAllByText("$175.20").length).toBeGreaterThan(0);
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "200" } });
  expect(screen.getAllByText("$350.40").length).toBeGreaterThan(0);
  fireEvent.click(screen.getByRole("button", { name: "By users" }));
  fireEvent.change(screen.getByLabelText("Active users"), { target: { value: "10" } });
  fireEvent.change(screen.getByLabelText("Tasks per user per day"), { target: { value: "3" } });
  expect(screen.getAllByText("$52.56").length).toBeGreaterThan(0);
  fireEvent.change(screen.getByLabelText("Active users slider"), { target: { value: "0" } });
  expect(screen.getAllByText("$0.00").length).toBeGreaterThan(0);
});
it("withholds empty volume and all-zero mix, and can recover", () => {
  render(<CostProjectionDashboard {...props} />);
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "" } });
  expect(screen.getByRole("alert")).toBeInTheDocument();
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "10" } });
  screen
    .getAllByLabelText("Frequency")
    .forEach((el) => fireEvent.change(el, { target: { value: "0" } }));
  expect(screen.getByRole("status")).toHaveTextContent(/select at least one/);
  fireEvent.change(screen.getAllByLabelText("Frequency")[0], { target: { value: "1" } });
  expect(screen.getByRole("img")).toBeInTheDocument();
});
it("edits workload assumptions and rejects invalid inputs", () => {
  render(<CostProjectionDashboard {...props} />);
  fireEvent.click(screen.getByText("Advanced assumptions"));
  fireEvent.change(screen.getByLabelText("Model calls per task"), { target: { value: "" } });
  expect(screen.queryByRole("img")).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Model calls per task"), { target: { value: "2" } });
  expect(screen.getByRole("img")).toBeInTheDocument();
});
it.each([
  ["uk", "Розумна"],
  ["fr", "Projection"],
  ["de", "Kosten"],
  ["es", "Proyección"],
  ["zh-CN", "预测"],
])("renders localized projection for %s", (locale) => {
  localStorage.setItem("everruns.ui-language", locale);
  render(
    <LocaleProvider>
      <CostProjectionDashboard {...props} />
    </LocaleProvider>,
  );
  expect(screen.queryByText("Typical tasks for your agent")).not.toBeInTheDocument();
  expect(screen.getAllByText(task.name).length).toBeGreaterThan(0);
});

it("keeps the slider scale stable after entering a large exact volume", () => {
  render(<CostProjectionDashboard {...props} />);
  fireEvent.change(screen.getByLabelText("Tasks per day"), { target: { value: "100000" } });
  const slider = screen.getByLabelText("Tasks per day slider");
  expect(slider).toHaveAttribute("max", "100000");
  fireEvent.change(slider, { target: { value: "50000" } });
  expect(slider).toHaveAttribute("max", "100000");
  expect(screen.getByLabelText("Tasks per day")).toHaveValue(50000);
});
