import { render, screen } from "@testing-library/react";
import { SessionApprovals } from "@/components/session/session-approvals";
import { buildApprovalEpisodes } from "@/lib/approval-episodes";
import type { Event } from "@/lib/api/types";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

function toolEvent(
  id: string,
  toolName: string,
  payload: Record<string, unknown>,
  sequence: number,
): Event {
  return {
    id,
    type: "tool.completed",
    ts: `2026-10-02T22:56:${String(sequence).padStart(2, "0")}Z`,
    sequence,
    session_id: "session_1",
    context: {},
    data: {
      tool_call_id: id,
      tool_name: toolName,
      success: true,
      status: "success",
      result: [{ type: "text", text: JSON.stringify(payload) }],
    },
  };
}

const ASK =
  'Create the reusable organisation-wide agent "SRE Simulator" using the built-in Generic harness and the proposed simulation-only instructions';
const GRANT =
  "Create the organisation-wide SRE Simulator agent using the built-in Generic harness and simulation-only instructions";

describe("SessionApprovals", () => {
  it("shows the request and the recorded grant on one card", () => {
    const episodes = buildApprovalEpisodes(
      [
        toolEvent(
          "ask",
          "request_approval",
          {
            action: ASK,
            question: "Create the SRE Simulator agent with the proposed safe configuration?",
          },
          1,
        ),
        toolEvent(
          "grant",
          "record_approval",
          {
            action: GRANT,
            detail: "Approved configuration: safety-first, simulation-only.",
            approved_in_message: "msg_consent",
          },
          2,
        ),
      ],
      { memberNames: new Map([["user_1", "Mykhailo Chalyi"]]) },
    );

    render(<SessionApprovals sessionId="session_1" episodes={episodes} />);

    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(1);
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent(GRANT);
    expect(screen.getByText("Approved")).toBeInTheDocument();
    expect(screen.getByText("Requested")).toBeInTheDocument();
    expect(screen.getByText(ASK)).toBeInTheDocument();
    expect(
      screen.getByText("Approved configuration: safety-first, simulation-only."),
    ).toBeInTheDocument();
    expect(screen.getByText(/Approved by an unknown actor/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /view consent/i })).toHaveAttribute(
      "href",
      "/sessions/session_1/trace",
    );
    expect(screen.getByText("1 approved")).toBeInTheDocument();
  });

  it("does not repeat an action the grant recorded in the same words", () => {
    const episodes = buildApprovalEpisodes([
      toolEvent(
        "ask",
        "request_approval",
        { action: "delete the bucket", question: "Delete it?" },
        1,
      ),
      toolEvent("grant", "record_approval", { action: "delete the bucket" }, 2),
    ]);

    render(<SessionApprovals sessionId="session_1" episodes={episodes} />);

    expect(screen.getAllByText("delete the bucket")).toHaveLength(1);
    expect(screen.getByText("Delete it?")).toBeInTheDocument();
  });

  it("shows an open request as still unanswered", () => {
    const episodes = buildApprovalEpisodes(
      [
        toolEvent(
          "ask",
          "request_approval",
          { action: "drop prod", question: "Drop production?" },
          1,
        ),
      ],
      { inputsComplete: true },
    );

    render(<SessionApprovals sessionId="session_1" episodes={episodes} />);

    expect(screen.getByText("Open")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("drop prod");
    expect(screen.getByText("Drop production?")).toBeInTheDocument();
    expect(
      screen.getByText("Waiting for consent. Nothing has been recorded as approved."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /view consent/i })).not.toBeInTheDocument();
  });

  it("says when a recording has no approvals", () => {
    render(<SessionApprovals sessionId="session_1" episodes={[]} />);

    expect(screen.getByText("No approvals in this session")).toBeInTheDocument();
  });
});
