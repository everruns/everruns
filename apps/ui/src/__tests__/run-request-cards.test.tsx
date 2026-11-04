import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RunRequestCards } from "@/components/chat/run-request-cards";

const getRunRequests = jest.fn();
const submitToolApprovals = jest.fn();

// Only approval cards render here; the connection card pulls in markdown.
jest.mock("@/components/chat/setup-connection-tool-call", () => ({
  SetupConnectionToolCall: () => null,
}));

jest.mock("@/lib/api/sessions", () => ({
  getRunRequests: (...args: unknown[]) => getRunRequests(...args),
  submitToolApprovals: (...args: unknown[]) => submitToolApprovals(...args),
}));

function approval(sessionId: string, agent: string) {
  return {
    session_id: sessionId,
    agent: { id: `agent_${agent}`, name: agent },
    requested_at: "2099-01-01T00:00:00Z",
    tool_call: {
      id: `tool_approval_${agent}`,
      name: "approve_tool_call",
      arguments: {
        tool: "git_push",
        display_name: "Push branch",
        arguments: { branch: "fix/login-refresh" },
        risk: "open_world",
        expires_at: "2099-01-01T00:15:00Z",
      },
    },
  };
}

function renderCards() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <RunRequestCards sessionId="session_first" sessionActive={false} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  getRunRequests.mockReset();
  submitToolApprovals.mockReset();
});

describe("RunRequestCards", () => {
  it("shows nothing while no other agent waits", async () => {
    getRunRequests.mockResolvedValue({ requests: [] });
    const { container } = renderCards();
    await waitFor(() => expect(getRunRequests).toHaveBeenCalledWith("session_first"));
    expect(container).toBeEmptyDOMElement();
  });

  it("heads each card with the asking agent and answers on its session", async () => {
    getRunRequests.mockResolvedValue({
      requests: [approval("session_impl", "Implement"), approval("session_review", "Review")],
    });
    submitToolApprovals.mockResolvedValue({
      status: "active",
      resolved: [
        {
          tool_call_id: "tool_approval_Implement",
          tool: "git_push",
          outcome: "allow",
        },
      ],
    });
    renderCards();

    expect(await screen.findByRole("region", { name: "Implement needs you" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Review needs you" })).toBeInTheDocument();
    expect(screen.getAllByText(/"branch": "fix\/login-refresh"/)).toHaveLength(2);

    fireEvent.click(screen.getAllByRole("button", { name: /allow once/i })[0]);
    await waitFor(() =>
      expect(submitToolApprovals).toHaveBeenCalledWith("session_impl", [
        { tool_call_id: "tool_approval_Implement", decision: "allow" },
      ]),
    );
  });
});
