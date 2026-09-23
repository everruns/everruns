import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { BudgetPanel } from "@/components/budgets/budget-panel";
import * as budgetsApi from "@/lib/api/budgets";
import type { Budget } from "@/lib/api/types";

jest.mock("@/lib/api/budgets");

const listBudgets = jest.mocked(budgetsApi.listBudgets);
const createBudget = jest.mocked(budgetsApi.createBudget);
const updateBudget = jest.mocked(budgetsApi.updateBudget);
const deleteBudget = jest.mocked(budgetsApi.deleteBudget);

const channelBudget: Budget = {
  id: "bdgt_channel",
  organization_id: "org_1",
  subject_type: "agent_channel",
  subject_id: "channel_1",
  currency: "usd",
  limit: 10,
  soft_limit: 8,
  balance: 0,
  period: { type: "duration", seconds: 3_600 },
  period_started_at: "2026-09-19T04:00:00.000Z",
  metadata: {
    converted_from: "app",
    converted_from_subject_id: "app_legacy",
  },
  status: "exhausted",
  created_at: "2026-09-19T04:00:00.000Z",
  updated_at: "2026-09-19T05:00:00.000Z",
};

function renderPanel(props?: Partial<React.ComponentProps<typeof BudgetPanel>>) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  return render(
    <BudgetPanel
      subjectType="agent_channel"
      subjectId="channel_1"
      title="Channel budget"
      canManage
      {...props}
    />,
    { wrapper },
  );
}

describe("BudgetPanel", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    listBudgets.mockResolvedValue([channelBudget]);
    createBudget.mockResolvedValue(channelBudget);
    updateBudget.mockResolvedValue({ ...channelBudget, limit: 20 });
    deleteBudget.mockResolvedValue();
  });

  it("shows the refusing cap, rollover state, and migrated App provenance", async () => {
    renderPanel();

    expect(await screen.findByText("bdgt_channel")).toBeInTheDocument();
    expect(screen.getByText("Migrated App cap")).toBeInTheDocument();
    expect(screen.getByText(/1h sliding/)).toBeInTheDocument();
    expect(screen.getByText(/Started Sep 19, 2026/)).toBeInTheDocument();
    expect(screen.getByText(/Reset due Sep 19, 2026/)).toBeInTheDocument();
    expect(screen.getByText("exhausted")).toBeInTheDocument();
    expect(listBudgets).toHaveBeenCalledWith({
      subject_type: "agent_channel",
      subject_id: "channel_1",
    });
  });
  it("keeps the cap visible but hides mutations without budget.manage", async () => {
    renderPanel({ canManage: false });

    expect(await screen.findByText("bdgt_channel")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add budget" })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Edit budget bdgt_channel" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Delete budget bdgt_channel" }),
    ).not.toBeInTheDocument();
  });

  it("creates a channel budget with the fixed channel subject", async () => {
    listBudgets.mockResolvedValue([]);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Add budget" }));
    fireEvent.change(screen.getByLabelText("Limit"), { target: { value: "25" } });
    fireEvent.click(screen.getByRole("button", { name: "Create budget" }));

    await waitFor(() =>
      expect(createBudget).toHaveBeenCalledWith({
        subject_type: "agent_channel",
        subject_id: "channel_1",
        currency: "usd",
        limit: 25,
        soft_limit: null,
        period: { type: "calendar", unit: "month" },
      }),
    );
  });

  it("edits and removes a channel budget", async () => {
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Edit budget bdgt_channel" }));
    fireEvent.change(screen.getByLabelText("Limit"), { target: { value: "20" } });
    fireEvent.click(screen.getByRole("button", { name: "Save budget" }));

    await waitFor(() =>
      expect(updateBudget).toHaveBeenCalledWith("bdgt_channel", {
        limit: 20,
        soft_limit: 8,
        status: "exhausted",
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Delete budget bdgt_channel" }));
    await waitFor(() => expect(deleteBudget).toHaveBeenCalledWith("bdgt_channel"));
  });
});
