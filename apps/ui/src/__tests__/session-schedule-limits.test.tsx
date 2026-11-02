import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { SessionScheduleList } from "@/components/session/session-schedule-list";
import { buildExtendRequest } from "@/components/session/session-schedule-extend-dialog";
import type { SessionSchedule } from "@/lib/api/types";

const mockMutate = jest.fn();
jest.mock("@/hooks/use-session-schedules", () => ({
  useSessionSchedules: jest.fn(),
  useUpdateSessionSchedule: () => ({ mutateAsync: mockMutate, isPending: false }),
}));

function schedule(overrides: Partial<SessionSchedule> = {}): SessionSchedule {
  return {
    id: "sched_1",
    session_id: "session_1",
    owner_principal_id: "user_1",
    description: "Poll the claim",
    cron_expression: "*/5 * * * *",
    timezone: "UTC",
    enabled: true,
    schedule_type: "recurring",
    next_trigger_at: "2026-11-01T12:05:00Z",
    trigger_count: 12,
    created_at: "2026-11-01T10:00:00Z",
    updated_at: "2026-11-01T11:00:00Z",
    ends_at: "2026-11-02T10:00:00Z",
    max_runs: 96,
    stop_when: "the claim returns an assertion",
    stop_reason: null,
    budget: { limit_usd: 2, spent_usd: 0.4, status: "active" },
    ...overrides,
  } as SessionSchedule;
}

beforeEach(() => {
  mockMutate.mockReset().mockResolvedValue({});
});

test("shows run count, end time, stop condition and budget", () => {
  render(<SessionScheduleList schedules={[schedule()]} />);
  expect(screen.getByText(/Run 12 of 96/)).toBeInTheDocument();
  expect(screen.getByText(/Ends Nov 2/)).toBeInTheDocument();
  expect(screen.getByText(/\$0\.40 of \$2\.00/)).toBeInTheDocument();
  expect(screen.getByText("Stops when: the claim returns an assertion")).toBeInTheDocument();
  expect(screen.getByText("Active")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Re-enable" })).not.toBeInTheDocument();
});

test("a stopped schedule shows why and can be re-enabled", async () => {
  render(
    <SessionScheduleList
      schedules={[
        schedule({ id: "sched_ended", enabled: false, stop_reason: "ended" }),
        schedule({ id: "sched_budget", enabled: false, stop_reason: "budget_exhausted" }),
      ]}
    />,
  );
  expect(screen.getByText("Ended")).toBeInTheDocument();
  expect(screen.getByText("Usage limit reached")).toBeInTheDocument();
  fireEvent.click(screen.getAllByRole("button", { name: "Re-enable" })[1]);
  await waitFor(() =>
    expect(mockMutate).toHaveBeenCalledWith({
      sessionId: "session_1",
      scheduleId: "sched_budget",
      request: { enabled: true },
    }),
  );
});

test("deleted schedules have no actions", () => {
  render(
    <SessionScheduleList
      schedules={[schedule({ enabled: false, deleted_at: "2026-11-01T11:30:00Z" })]}
    />,
  );
  expect(screen.getByText("Deleted")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Extend" })).not.toBeInTheDocument();
});

test("Extend sends only the values the person changed", async () => {
  render(<SessionScheduleList schedules={[schedule()]} />);
  fireEvent.click(screen.getByRole("button", { name: "Extend" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.change(within(dialog).getByLabelText("Most runs"), { target: { value: "200" } });
  fireEvent.change(within(dialog).getByLabelText("Budget (USD)"), { target: { value: "15" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Extend" }));
  await waitFor(() =>
    expect(mockMutate).toHaveBeenCalledWith({
      sessionId: "session_1",
      scheduleId: "sched_1",
      request: { max_runs: 200, budget_usd: 15 },
    }),
  );
});

test("buildExtendRequest validates the end time and requires a change", () => {
  const current = schedule();
  const now = new Date("2026-11-01T12:00:00Z");
  expect(buildExtendRequest(current, { endsAt: "", maxRuns: "96", budgetUsd: "2" }, now)).toBe(
    "Change at least one value to extend.",
  );
  expect(
    buildExtendRequest(current, { endsAt: "2020-01-01T00:00", maxRuns: "", budgetUsd: "" }, now),
  ).toBe("The end time must be in the future.");
  const later = buildExtendRequest(
    current,
    { endsAt: "2026-11-05T10:00", maxRuns: "", budgetUsd: "" },
    now,
  );
  expect(later).toEqual({ ends_at: new Date("2026-11-05T10:00").toISOString() });
});
