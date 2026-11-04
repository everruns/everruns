import { render, screen } from "@testing-library/react";
import { SessionCard } from "@/components/session/session-card";
import { RunBars } from "@/components/agents/home/run-bars";
import { agentFailureHref, failureTraceHref } from "@/lib/agent-failures";
import { sessionOutcome } from "@/lib/session-outcome";
import type { Session } from "@/lib/api/types";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href, ...props }: React.ComponentProps<"a">) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

const session: Session = {
  id: "session_failure",
  organization_id: "org_default",
  harness_id: "harness_default",
  agent_id: "agent_release",
  owner_principal_id: "principal_owner",
  title: "Release monitoring",
  tags: [],
  model_id: null,
  status: "idle",
  activity: "failed",
  last_turn_status: "failed",
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-16T09:42:00Z",
  started_at: null,
  finished_at: null,
  output_preview: "The earlier turn succeeded.",
};

it("keeps the chart's exact snapshot window in the failure-count link", () => {
  const before = "2026-10-16T09:42:00.123Z";
  const url = new URL(agentFailureHref("agent_release", before), "https://example.com");
  expect(url.pathname).toBe("/agents/agent_release");
  expect(Object.fromEntries(url.searchParams)).toEqual({
    tab: "sessions",
    failures: "true",
    after: "2026-10-15T09:42:00.123Z",
    before,
  });
});

it("preserves server timestamp precision at the failure-window boundary", () => {
  const before = "2026-10-16T09:42:00.123456Z";
  const after = "2026-10-15T09:42:00.123456Z";
  const url = new URL(agentFailureHref("agent_release", before, after), "https://example.com");
  expect(url.searchParams.get("after")).toBe(after);
  expect(url.searchParams.get("before")).toBe(before);
});

it("shows a historical failure beside the session's recovered outcome", () => {
  render(
    <SessionCard
      session={{ ...session, activity: "completed", last_turn_status: "completed" }}
      failure={{ count: 2, at: session.updated_at, error: "Model request timed out", sequence: 42 }}
    />,
  );
  expect(screen.getByText("Completed")).toBeInTheDocument();
  expect(screen.getByText(/2 failed turns/)).toBeInTheDocument();
  expect(screen.getByText("Model request timed out")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "View failure" })).toHaveAttribute(
    "href",
    failureTraceHref(session.id, 42),
  );
  expect(screen.queryByText(session.output_preview!)).not.toBeInTheDocument();
});

it("shows terminal failure instead of idle, with an honest missing-error fallback", () => {
  render(
    <SessionCard session={session} failure={{ count: 1, at: session.updated_at, sequence: 42 }} />,
  );
  expect(screen.getByText("Last turn failed")).toBeInTheDocument();
  expect(screen.queryByText("Idle")).not.toBeInTheDocument();
  expect(screen.getByText("No error details recorded")).toBeInTheDocument();
});

it("distinguishes cancellation and gives live execution precedence over history", () => {
  expect(sessionOutcome({ ...session, last_turn_status: "cancelled" }).label).toBe("Stopped");
  expect(sessionOutcome(session, "active").label).toBe("Running");
  expect(sessionOutcome(session, "waiting_for_tool_results").label).toBe("Waiting on tools");
  expect(sessionOutcome(session, "paused").label).toBe("Paused");
});

it("draws a failure even when the turn started outside the chart window", () => {
  render(<RunBars buckets={[{ runs: 0, failed: 1 }]} label="No turns started; one failed" />);
  const bar = screen.getByRole("img").firstElementChild;
  expect(bar).toHaveClass("bg-destructive");
  expect(bar).toHaveStyle({ height: "100%" });
});
