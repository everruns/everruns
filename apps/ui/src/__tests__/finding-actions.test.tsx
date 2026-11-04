// What was done about a finding: an automatic pause offers Resume, an offered
// pause offers Pause schedule, a resumed pause offers nothing, and a viewer who
// cannot manage agents sees the record without buttons.

import { fireEvent, render, screen } from "@testing-library/react";
import { FindingActionsCard } from "@/components/evals/finding-actions";
import type { FindingActions } from "@/lib/api/types";

const mutate = jest.fn();
let mockData: FindingActions | undefined;
let mockCanManage = true;
jest.mock("@/hooks/use-evals", () => ({
  useFindingActions: () => ({ data: mockData }),
  useFindingActionMutation: () => ({ mutate, isPending: false, error: null }),
}));
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => mockCanManage }),
}));

const paused = {
  id: "fact_1",
  kind: "pause_schedule",
  target_id: "sched_1",
  automatic: true,
  current: true,
  created_at: "2026-11-02T10:00:00Z",
} as FindingActions["actions"][number];

beforeEach(() => {
  mutate.mockReset();
  mockCanManage = true;
});

it("shows an automatic pause with Resume", () => {
  mockData = { actions: [paused], available: [], mode: "auto" } as FindingActions;
  render(<FindingActionsCard issueId="issue-1" />);
  expect(screen.getByText("Done automatically")).toBeInTheDocument();
  expect(screen.getByText("Paused the schedule")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /Resume schedule/ }));
  expect(mutate).toHaveBeenCalledWith({ kind: "undo", actionId: "fact_1" });
});

it("offers the pause when the policy only suggests it", () => {
  mockData = {
    actions: [],
    available: [{ kind: "pause_schedule", target_id: "sched_1" }],
    mode: "suggest",
  } as FindingActions;
  render(<FindingActionsCard issueId="issue-1" />);
  expect(screen.getByText("Suggested action")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /Pause schedule/ }));
  expect(mutate).toHaveBeenCalledWith({ kind: "take", actionKind: "pause_schedule" });
});

it("records a resumed pause without buttons", () => {
  mockData = {
    actions: [{ ...paused, undone_at: "2026-11-02T11:00:00Z" }],
    available: [],
    mode: "auto",
  } as FindingActions;
  render(<FindingActionsCard issueId="issue-1" />);
  expect(screen.getByText("The schedule was paused, then resumed.")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("hides the buttons from people who cannot manage agents", () => {
  mockCanManage = false;
  mockData = { actions: [paused], available: [], mode: "auto" } as FindingActions;
  render(<FindingActionsCard issueId="issue-1" />);
  expect(screen.getByText("Paused the schedule")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("renders nothing when nothing was done or offered", () => {
  mockData = { actions: [], available: [], mode: "off" } as FindingActions;
  const { container } = render(<FindingActionsCard issueId="issue-1" />);
  expect(container).toBeEmptyDOMElement();
});
