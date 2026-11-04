// Eval Builder proposals on an agent's Evals tab: each says how it runs and
// where it came from, Add goes straight into the agent's only dataset, and the
// card explains a run that is going, failed or found nothing.

import { fireEvent, render, screen } from "@testing-library/react";
import { EvalProposalsCard, proposalMeta, runNotice } from "@/components/evals/eval-proposals";
import type { Eval, EvalBuilderRun, EvalCaseProposal } from "@/lib/api/types";

const accept = jest.fn();
const dismiss = jest.fn();
jest.mock("@/hooks/use-evals", () => ({
  useAcceptEvalProposal: () => ({
    mutate: accept,
    mutateAsync: accept,
    isPending: false,
  }),
  useDismissEvalProposal: () => ({
    mutate: dismiss,
    mutateAsync: dismiss,
    isPending: false,
  }),
}));

const ui = (key: string, params?: Record<string, string | number>) =>
  key.replace(/\{(\w+)\}/g, (_, k: string) => String(params?.[k] ?? `{${k}}`));

function proposal(overrides: Partial<EvalCaseProposal> = {}): EvalCaseProposal {
  return {
    id: "evalprop_1",
    agent_id: "agent_1",
    status: "proposed",
    name: "Refund over the approval limit",
    category: "edge_case",
    kind: "scripted",
    reason: "The instructions cap refunds at $200 and no case tests it.",
    source_session_ids: [],
    case: {
      name: "Refund over the approval limit",
      conversation: [{ content: "Refund $450 for order 1182" }],
      scorers: [{ type: "decision", question: "Does it hand the refund to a person?" }],
    },
    created_at: "2026-11-02T10:00:00Z",
    ...overrides,
  } as EvalCaseProposal;
}

function run(overrides: Partial<EvalBuilderRun> = {}): EvalBuilderRun {
  return {
    id: "evalbuild_1",
    status: "done",
    started_at: "2026-11-02T10:00:00Z",
    sessions_read: 3,
    proposals: 2,
    ...overrides,
  } as EvalBuilderRun;
}

const dataset = { id: "eval_1", name: "Refunds" } as Eval;

beforeEach(() => {
  accept.mockReset();
  dismiss.mockReset();
});

describe("Eval Builder proposals", () => {
  it("says how a case runs, what it covers and where it came from", () => {
    expect(proposalMeta(proposal(), ui)).toBe("Scripted · edge case · from instructions");
    expect(
      proposalMeta(
        proposal({
          kind: "simulated",
          category: "failure",
          source_session_ids: ["s1", "s2"],
        }),
        ui,
      ),
    ).toBe("Simulated · seen failing · seen in 2 sessions");
  });

  it("explains a run in progress, a failure, and a run that found nothing", () => {
    expect(runNotice(undefined, 0, ui)).toBeNull();
    expect(runNotice(run({ status: "running" }), 0, ui)).toMatch(/Reading the agent/);
    expect(runNotice(run({ status: "unavailable" }), 0, ui)).toMatch(/default model/);
    expect(runNotice(run({ status: "failed", error: "interrupted" }), 0, ui)).toBe("interrupted");
    expect(runNotice(run({ proposals: 0 }), 0, ui)).toMatch(/No new cases/);
    expect(runNotice(run(), 2, ui)).toBeNull();
  });

  it("adds straight into the agent's only dataset, with the reason recorded", () => {
    render(
      <EvalProposalsCard
        agentId="agent_1"
        proposals={[proposal()]}
        latestRun={run()}
        datasets={[dataset]}
      />,
    );
    expect(screen.getByText("1 new")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(accept).toHaveBeenCalledWith({
      proposalId: "evalprop_1",
      evalId: "eval_1",
      reason: expect.stringContaining("no case tests it"),
    });
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(dismiss).toHaveBeenCalledWith({ proposalId: "evalprop_1" });
  });

  it("cannot add without a dataset and says so", () => {
    render(<EvalProposalsCard agentId="agent_1" proposals={[proposal()]} datasets={[]} />);
    expect(screen.getByRole("button", { name: "Add" })).toBeDisabled();
    expect(screen.getByText(/Create a dataset for this agent/)).toBeInTheDocument();
  });

  it("shows three on the card and the rest behind Review all", () => {
    const many = [1, 2, 3, 4].map((n) => proposal({ id: `evalprop_${n}`, name: `Case ${n}` }));
    render(<EvalProposalsCard agentId="agent_1" proposals={many} datasets={[dataset]} />);
    expect(screen.queryByText("Case 4")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Review all 4" }));
    expect(screen.getByText("Case 4")).toBeInTheDocument();
  });

  it("renders nothing before the builder was ever asked", () => {
    const { container } = render(
      <EvalProposalsCard agentId="agent_1" proposals={[]} datasets={[dataset]} />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});
