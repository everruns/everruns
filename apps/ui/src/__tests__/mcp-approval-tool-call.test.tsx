import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { McpApprovalToolCall } from "@/components/chat/mcp-approval-tool-call";
import type { ToolCompletedData } from "@/lib/api/types";

const submitToolResults = jest.fn();

jest.mock("@/lib/api/sessions", () => ({
  submitToolResults: (...args: unknown[]) => submitToolResults(...args),
}));

const approval = {
  server_label: "deepwiki",
  name: "ask_question",
  arguments: '{"repoName":"everruns/everruns"}',
};

function renderCard(results = new Map<string, ToolCompletedData>()) {
  return render(
    <McpApprovalToolCall
      sessionId="session_1"
      toolCallId="mcpr_1"
      approval={approval}
      toolResultsMap={results}
    />,
  );
}

beforeEach(() => {
  submitToolResults.mockReset();
  submitToolResults.mockResolvedValue({ status: "running", tool_results_count: 1 });
});

describe("McpApprovalToolCall", () => {
  it("shows the server, tool and arguments before anything runs", () => {
    renderCard();
    expect(screen.getByText("Allow deepwiki to run ask_question?")).toBeInTheDocument();
    expect(screen.getByText(/"repoName": "everruns\/everruns"/)).toBeInTheDocument();
    expect(submitToolResults).not.toHaveBeenCalled();
  });

  it("approves through the tool result", async () => {
    renderCard();
    fireEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() =>
      expect(submitToolResults).toHaveBeenCalledWith("session_1", [
        { tool_call_id: "mcpr_1", result: { approve: true } },
      ]),
    );
    expect(await screen.findByText("Approved ask_question on deepwiki")).toBeInTheDocument();
  });

  it("denies through the tool result", async () => {
    renderCard();
    fireEvent.click(screen.getByRole("button", { name: /deny/i }));
    await waitFor(() =>
      expect(submitToolResults).toHaveBeenCalledWith("session_1", [
        { tool_call_id: "mcpr_1", result: { approve: false } },
      ]),
    );
    expect(await screen.findByText("Denied ask_question on deepwiki")).toBeInTheDocument();
  });

  it("shows a recorded answer instead of the buttons", () => {
    const results = new Map<string, ToolCompletedData>([
      [
        "mcpr_1",
        {
          tool_call_id: "mcpr_1",
          tool_name: "openai_mcp_approval",
          success: true,
          status: "success",
          result: [{ type: "text", text: '{"approve":true}' }],
        } as ToolCompletedData,
      ],
    ]);
    renderCard(results);
    expect(screen.getByText("Approved ask_question on deepwiki")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});

it.each(['{"file\\u2066":"report\\u202Etxt.exe"}', "report\u202Etxt.exe\r\u001b"])(
  "reveals controls after JSON parsing or raw fallback and preserves MCP decisions: %s",
  async (raw) => {
    const input = { name: "run\u202Etool", server_label: "docs\u2069", arguments: raw };
    const original = JSON.stringify(input);
    const { container } = render(
      <McpApprovalToolCall
        sessionId="session_1"
        toolCallId="mcpr_bidi"
        approval={input}
        toolResultsMap={new Map()}
      />,
    );
    expect(container.textContent).not.toMatch(/[\u202E\u2066\u2069\r\u001b]/u);
    expect(screen.getAllByRole("img", { name: "Unicode control character U+202E" })).toHaveLength(
      2,
    );
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    await waitFor(() =>
      expect(submitToolResults).toHaveBeenCalledWith("session_1", [
        { tool_call_id: "mcpr_bidi", result: { approve: false } },
      ]),
    );
    expect(JSON.stringify(input)).toBe(original);
    expect(container.textContent).not.toMatch(/[\u202E\u2069]/u);
    expect(screen.getAllByRole("img")).toHaveLength(2);
  },
);
