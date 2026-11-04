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

it("shows credential destination and owner with only exact-request consent", async () => {
  render(
    <ToolApprovalRequests
      sessionId="session_1"
      requests={[
        {
          id: "tool_approval_capture",
          arguments: {
            tool: "web_fetch",
            risk: "credential:ticket",
            expires_at: "2099-01-01T00:15:00Z",
            arguments: {
              source: "https://auth.example/token",
              credentials: [
                {
                  name: "MAIL_TOKEN",
                  scope: "virtual_user",
                  owner_label: "Alice",
                  binding: { url: "https://mail.example/api", auth: { type: "bearer" } },
                  replacing: true,
                },
              ],
            },
          },
        },
      ]}
      toolResultsMap={new Map()}
    />,
  );
  expect(screen.getByText("https://auth.example/token")).toBeInTheDocument();
  expect(screen.getByText("https://mail.example/api")).toBeInTheDocument();
  expect(screen.getByText("Alice")).toBeInTheDocument();
  expect(screen.getByText("Replaces existing value")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /always/i })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Approve capture" }));
  await waitFor(() =>
    expect(submitToolApprovals).toHaveBeenCalledWith("session_1", [
      { tool_call_id: "tool_approval_capture", decision: "allow" },
    ]),
  );
});

it("reveals bidi in names and nested arguments without changing the submitted decision", async () => {
  const pending = {
    id: "tool_approval_bidi",
    arguments: {
      tool: "shell",
      display_name: "Run\u202Eshell",
      arguments: { "file\u2066": ["report\u202Etxt.exe", "می‌خواهم 👩‍💻"] },
      risk: "destructive",
    },
  };
  const original = JSON.stringify(pending);
  const { container } = render(
    <ToolApprovalRequests sessionId="session_1" requests={[pending]} toolResultsMap={new Map()} />,
  );
  expect(container.textContent).not.toMatch(/[\u202E\u2066]/u);
  expect(screen.getAllByRole("img", { name: "Unicode control character U+202E" })).toHaveLength(2);
  expect(container.querySelector("pre")).toHaveTextContent("می‌خواهم 👩‍💻");
  fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
  await waitFor(() =>
    expect(submitToolApprovals).toHaveBeenCalledWith("session_1", [
      { tool_call_id: "tool_approval_bidi", decision: "allow" },
    ]),
  );
  expect(JSON.stringify(pending)).toBe(original);
  expect(container.textContent).not.toContain("\u202E");
  expect(
    await screen.findByRole("img", { name: "Unicode control character U+202E" }),
  ).toBeInTheDocument();
});

it("reveals controls in raw arguments and fallback tool names", () => {
  const { container } = render(
    <ToolApprovalRequests
      sessionId="session_1"
      requests={[
        {
          id: "raw",
          arguments: { tool: "shell\u200F", arguments: "echo safe\r\u001b\u202Eevil" },
        },
      ]}
      toolResultsMap={new Map()}
    />,
  );
  expect(container.textContent).not.toMatch(/[\u200F\r\u001b\u202E]/u);
  expect(container.querySelector("pre")?.textContent).toBe("echo safe\\u000D\\u001B\\u202Eevil");
});

it("reveals controls in credential consent fields and the exact request", () => {
  const { container } = render(
    <ToolApprovalRequests
      sessionId="session_1"
      requests={[
        {
          id: "credential",
          arguments: {
            risk: "credential:ticket",
            arguments: {
              source: "https://auth.example/\u202Etoken",
              method: "GET\u200F",
              request: { path: "\u2066" },
              credentials: [
                {
                  name: "TOKEN\u202C",
                  from: "token\u2069",
                  scope: "custom\u061C",
                  owner_label: "Alice\u202E",
                  binding: {
                    url: "https://example.com/\u202Eapi",
                    auth: { type: "header", name: "X-Key\u2068" },
                  },
                },
              ],
            },
          },
        },
      ]}
      toolResultsMap={new Map()}
    />,
  );
  expect(container.textContent).not.toMatch(/[\u061C\u200F\u202C\u202E\u2066\u2068\u2069]/u);
  expect(container.querySelector("pre")?.textContent).toContain("\\u2066");
  expect(screen.getAllByRole("img", { name: "Unicode control character U+202E" })).toHaveLength(3);
});
