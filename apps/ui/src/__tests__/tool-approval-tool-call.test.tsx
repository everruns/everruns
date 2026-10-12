import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ToolApprovalRequests } from "@/components/chat/tool-approval-tool-call";
import type { ToolCompletedData } from "@/lib/api/types";

const submitToolApprovals = jest.fn();

jest.mock("@/lib/api/sessions", () => ({
  submitToolApprovals: (...args: unknown[]) => submitToolApprovals(...args),
}));

function request(id: string, to: string) {
  return {
    id: `tool_approval_${id}`,
    arguments: {
      tool: "send_email",
      display_name: "Send email",
      arguments: { to },
      risk: "open_world",
      expires_at: "2099-01-01T00:15:00Z",
    },
  };
}

function renderCards(
  requests = [request("call_1", "cfo@example.com")],
  results = new Map<string, ToolCompletedData>(),
) {
  return render(
    <ToolApprovalRequests sessionId="session_1" requests={requests} toolResultsMap={results} />,
  );
}

beforeEach(() => {
  submitToolApprovals.mockReset();
  submitToolApprovals.mockImplementation(
    async (_session: string, decisions: Array<{ tool_call_id: string; decision: string }>) => ({
      status: "active",
      resolved: decisions.map((d) => ({
        tool_call_id: d.tool_call_id,
        tool: "send_email",
        outcome: d.decision,
      })),
    }),
  );
});

describe("ToolApprovalRequests", () => {
  it("shows the tool, its risk and the exact arguments before anything runs", () => {
    renderCards();
    expect(screen.getByText("Allow the agent to run Send email?")).toBeInTheDocument();
    expect(screen.getByText(/reaches outside this session/)).toBeInTheDocument();
    expect(screen.getByText(/"to": "cfo@example.com"/)).toBeInTheDocument();
    expect(submitToolApprovals).not.toHaveBeenCalled();
  });

  it("says when only a rating, not the tool, asked for approval", () => {
    const rated = request("call_1", "cfo@example.com");
    rated.arguments.risk = "rated_changes";
    renderCards([rated]);
    expect(screen.getByText(/says nothing about its risk/)).toBeInTheDocument();
  });

  it("submits a single decision straight away", async () => {
    renderCards();
    fireEvent.click(screen.getByRole("button", { name: /allow once/i }));
    await waitFor(() =>
      expect(submitToolApprovals).toHaveBeenCalledWith("session_1", [
        { tool_call_id: "tool_approval_call_1", decision: "allow" },
      ]),
    );
    expect(await screen.findByText("Allowed once: Send email")).toBeInTheDocument();
  });

  it("waits for every request in a batch before submitting", async () => {
    renderCards([request("call_1", "a@example.com"), request("call_2", "b@example.com")]);
    fireEvent.click(screen.getAllByRole("button", { name: /^reject$/i })[0]);
    expect(submitToolApprovals).not.toHaveBeenCalled();
    fireEvent.click(screen.getAllByRole("button", { name: /always allow/i })[1]);
    await waitFor(() =>
      expect(submitToolApprovals).toHaveBeenCalledWith("session_1", [
        { tool_call_id: "tool_approval_call_1", decision: "reject" },
        { tool_call_id: "tool_approval_call_2", decision: "allow_always" },
      ]),
    );
  });

  it("shows a recorded outcome instead of the buttons", () => {
    const results = new Map<string, ToolCompletedData>([
      [
        "tool_approval_call_1",
        {
          tool_call_id: "tool_approval_call_1",
          tool_name: "approve_tool_call",
          success: true,
          status: "success",
          result: [{ type: "text", text: '{"outcome":"expired","approved":false}' }],
        } as ToolCompletedData,
      ],
    ]);
    renderCards(undefined, results);
    expect(screen.getByText("Not approved in time: Send email")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
