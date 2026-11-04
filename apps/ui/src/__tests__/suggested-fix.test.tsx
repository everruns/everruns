// The finding's suggested fix: Approve appears only after a clean check
// against the instructions the agent has now; before that the card offers the
// check, and a decided fix offers nothing.

import { fireEvent, render, screen } from "@testing-library/react";
import { SuggestedFix, fixBadge } from "@/components/evals/suggested-fix";
import type { FindingFix } from "@/lib/api/types";

const mutate = jest.fn();
let mockFix: FindingFix | null = null;
jest.mock("@/hooks/use-evals", () => ({
  useFindingFix: () => ({ data: mockFix }),
  useFindingFixAction: () => ({ mutate, isPending: false, error: null }),
}));
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => true }),
}));

function fix(overrides: Partial<FindingFix> = {}): FindingFix {
  return {
    id: "fix_1",
    issue_id: "issue-1",
    agent_id: "agent_1",
    summary: "Add a stop rule to the schedule's instructions",
    addition:
      "Stop and delete this schedule once the claim is approved, expired, or after 12 runs.",
    status: "proposed",
    current: true,
    created_at: "2026-11-02T10:00:00Z",
    ...overrides,
  } as FindingFix;
}

const check = {
  eval_name: "Claims",
  dataset_version: 2,
  trials: 2,
  cases: 5,
  current: { passed: 4, pass_rate: 0.8, cost_usd: 0.4 },
  with_fix: { passed: 5, pass_rate: 1, cost_usd: 0.42 },
  fixed: 1,
  broke: 0,
  verdict: "within_noise",
} as FindingFix["check"];

beforeEach(() => mutate.mockReset());

it("offers the check, not Approve, for a fix nobody checked", () => {
  mockFix = fix();
  render(<SuggestedFix issueId="issue-1" />);
  expect(screen.getByText(/\+ Stop and delete this schedule/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Approve/ })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check against evals" }));
  expect(mutate).toHaveBeenCalledWith({ kind: "check", fixId: "fix_1" });
});

it("approves a fix whose check found no regressions", () => {
  mockFix = fix({ status: "verified", check });
  render(<SuggestedFix issueId="issue-1" />);
  expect(screen.getByText("Needs approval")).toBeInTheDocument();
  expect(screen.getByText(/Checked on Claims v2: 4\/5 → 5\/5/)).toBeInTheDocument();
  expect(screen.getByText(/no regressions/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /Approve/ }));
  expect(mutate).toHaveBeenCalledWith(expect.objectContaining({ kind: "apply", fixId: "fix_1" }));
});

it("asks for another check when the instructions changed after it", () => {
  mockFix = fix({ status: "verified", current: false, check });
  render(<SuggestedFix issueId="issue-1" />);
  expect(screen.queryByRole("button", { name: /Approve/ })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Check again" })).toBeInTheDocument();
  expect(fixBadge(mockFix).label).toBe("Check again");
});

it("offers nothing once a fix is applied or rejected", () => {
  mockFix = fix({ status: "rejected", decision_note: "Fixing the schedule itself" });
  render(<SuggestedFix issueId="issue-1" />);
  expect(screen.getByText("Rejected: Fixing the schedule itself")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("shows nothing when the Inspector suggested no fix", () => {
  mockFix = null;
  const { container } = render(<SuggestedFix issueId="issue-1" />);
  expect(container).toBeEmptyDOMElement();
});
