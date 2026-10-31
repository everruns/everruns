import { render, screen } from "@testing-library/react";
import {
  WorkPlanPinned,
  WorkResultBox,
  workFactsLine,
  workReportStatus,
} from "@/components/chat/work-report-parts";
import type { WorkReport } from "@/lib/api/types";

jest.mock("@/providers/locale-provider", () => {
  const { formatMessage } = jest.requireActual("@/lib/i18n");
  return {
    ...jest.requireActual("@/providers/locale-provider"),
    useLocale: () => ({
      locale: "en",
      t: (key: string, values?: Record<string, string | number>) =>
        formatMessage("en", key, values),
    }),
  };
});

function report(overrides: Partial<WorkReport>): WorkReport {
  return {
    turn_id: "turn_1",
    state: "working",
    headline: "Working",
    steps: [],
    ...overrides,
  } as WorkReport;
}

const t = (key: string) =>
  ({
    work_needs_approval: "Needs your approval",
    work_waiting_answer: "Waiting for your answer",
    work_waiting_you: "Waiting for you",
  })[key] ?? key;

describe("work report status", () => {
  it("uses the headline only when it names a running step or a plan item", () => {
    expect(workReportStatus(report({ headline: "Working" }), t)).toBeUndefined();
    expect(
      workReportStatus(
        report({
          headline: "Running tests 7 of 12",
          steps: [
            {
              id: "c1",
              tool_name: "bash",
              label: "Running tests",
              state: "running",
              started_at: "2026-10-23T00:00:00Z",
            },
          ],
        } as Partial<WorkReport>),
        t,
      ),
    ).toBe("Running tests 7 of 12");
  });

  it("names what the work waits on, localized", () => {
    expect(
      workReportStatus(
        report({
          state: "needs_you",
          attention: {
            kind: "approval",
            prompt: "Deploy to staging",
            tool_call_ids: ["c1"],
          },
        }),
        t,
      ),
    ).toBe("Needs your approval · Deploy to staging");
    expect(
      workReportStatus(
        report({
          state: "needs_you",
          attention: { kind: "question", prompt: "", tool_call_ids: [] },
        }),
        t,
      ),
    ).toBe("Waiting for your answer");
    expect(workReportStatus(report({ state: "done" }), t)).toBeUndefined();
  });
});

describe("WorkPlanPinned", () => {
  it("shows the step the plan is on and every item", () => {
    render(
      <WorkPlanPinned
        plan={{
          items: [
            { title: "Find the bug", status: "done" },
            { title: "Fix the rounding", status: "in_progress" },
            { title: "Open a pull request", status: "pending" },
          ],
          done: 1,
          total: 3,
          current: "Fixing the rounding",
        }}
      />,
    );
    expect(screen.getByTestId("work-plan")).toHaveTextContent("Step 2 of 3 · Fixing the rounding");
    expect(screen.getByText("Open a pull request")).toBeInTheDocument();
  });
});

describe("WorkResultBox", () => {
  it("says what got done and links artifacts", () => {
    render(
      <WorkResultBox
        report={report({
          state: "done",
          result: {
            summary: "Fixed proration rounding.",
            summary_source: "agent",
            facts: [
              { kind: "files_changed", value: 2 },
              { kind: "issues", value: 1 },
            ],
            artifacts: [
              {
                name: "PR #4861",
                type: "pr",
                url: "https://github.com/o/r/pull/4861",
              },
              {
                name: "invoice.ts",
                type: "file",
                path: "/workspace/invoice.ts",
              },
            ],
          },
        })}
      />,
    );
    const box = screen.getByTestId("work-result");
    expect(box).toHaveTextContent("Done · 2 files changed");
    expect(box).toHaveTextContent("Fixed proration rounding.");
    expect(screen.getByRole("link", { name: /PR #4861/ })).toHaveAttribute(
      "href",
      "https://github.com/o/r/pull/4861",
    );
    expect(screen.getByText("invoice.ts")).toBeInTheDocument();
  });

  it("does not repeat a summary taken from the reply, and stays hidden with nothing to add", () => {
    const { container } = render(
      <WorkResultBox
        report={report({
          state: "done",
          result: {
            summary: "Saved the notes.",
            summary_source: "reply",
            facts: [],
          },
        })}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows what the agent checked, and a failure it reported", () => {
    render(
      <WorkResultBox
        report={report({
          state: "done",
          result: {
            summary: "The staging key is missing, so nothing was deployed.",
            summary_source: "agent",
            checks: "ran the deploy dry run",
            outcome: "failed",
            facts: [],
          },
        })}
      />,
    );
    const box = screen.getByTestId("work-result");
    expect(box).toHaveTextContent("Could not finish");
    expect(box).not.toHaveTextContent("Done");
    expect(box).toHaveTextContent("Checked: ran the deploy dry run");
  });

  it("never links a non-web URL", () => {
    render(
      <WorkResultBox
        report={report({
          state: "done",
          result: {
            facts: [],
            artifacts: [{ name: "bad", type: "url", url: "javascript:alert(1)" }],
          },
        })}
      />,
    );
    expect(screen.queryByRole("link")).toBeNull();
    expect(screen.getByText("bad")).toBeInTheDocument();
  });
});

describe("workFactsLine", () => {
  it("pluralizes in Ukrainian", () => {
    const done = report({
      state: "done",
      result: { facts: [{ kind: "files_changed", value: 3 }] },
    });
    expect(workFactsLine(done, "uk")).toBe("Змінено 3 файли");
    expect(workFactsLine(done, "en")).toBe("3 files changed");
  });
});
