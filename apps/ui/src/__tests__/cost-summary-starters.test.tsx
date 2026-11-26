import { render, screen, within } from "@testing-library/react";
import { CostsOverviewView } from "@/components/costs/costs-overview";
import type { CostSummary, CostSummaryLine } from "@/lib/api/costs";

const line = (key: string, amount: number): CostSummaryLine => ({
  key,
  basis: "provider",
  cost_usd: amount,
  records: 1,
  unpriced_records: 0,
  quantity: 0,
});
const summary: CostSummary = {
  from: "2026-11-01T00:00:00Z",
  to: "2026-12-01T00:00:00Z",
  timezone: "UTC",
  cost_usd: 9.61,
  unpriced_records: 0,
  by_basis: [line("provider", 9.61)],
  by_source: [],
  by_agent: [],
  by_user: [],
  by_day: [],
  by_started_by: [line("unattended", 6.61), line("person", 2), line("unknown", 1)],
  runs_with_agents: [],
};

it("keeps unrecorded starters separate from people in the cost table", () => {
  render(<CostsOverviewView summary={summary} />);
  const section = screen
    .getByRole("heading", { name: "By who started the work" })
    .closest("section")!;
  const table = within(section);
  expect(table.getAllByText("People")).toHaveLength(1);
  expect(table.getByRole("row", { name: "Unknown 1 $1.00" })).toBeInTheDocument();
  expect(
    table.getByText("Unattended work includes schedules and agent triggers."),
  ).toBeInTheDocument();
});
