import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import SessionApprovalsPage from "@/app/(main)/sessions/[sessionId]/approvals/page";
import type { Event, ToolCompletedData } from "@/lib/api/types";

const mockSubmit = jest.fn();
const mockContext = {
  sessionId: "session_test",
  events: [] as Event[],
  eventsLoading: false,
  hasMoreEvents: false,
  effectiveStatus: "waiting_for_tool_results",
  toolResultsMap: new Map<string, ToolCompletedData>(),
};
jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => mockContext,
}));
jest.mock("@/hooks/use-session-approvals", () => ({
  useSessionApprovals: () => ({ episodes: [], loading: false, retry: jest.fn() }),
}));
jest.mock("@/lib/api/sessions", () => ({
  submitToolApprovals: (...args: unknown[]) => mockSubmit(...args),
}));
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

beforeEach(() => {
  mockSubmit.mockReset();
  mockContext.effectiveStatus = "active";
  mockContext.toolResultsMap = new Map();
  mockContext.events = [
    {
      id: "event_request",
      session_id: mockContext.sessionId,
      type: "tool.call_requested",
      ts: "2026-10-19T00:00:00Z",
      context: {},
      data: {
        tool_calls: [
          {
            id: "tool_approval_call",
            name: "approve_tool_call",
            arguments: {
              tool: "web_fetch",
              tool_call_id: "call",
              fingerprint: "credential:ticket",
              risk: "credential:ticket",
              expires_at: "2099-01-01T00:00:00Z",
              arguments: {
                source: "https://mail.example/token",
                method: "POST",
                acting_as: "user",
                credentials: [
                  {
                    name: "MAIL_TOKEN",
                    from: "/access_token",
                    scope: "virtual_user",
                    owner_label: "Alice",
                    binding: { url: "https://mail.example/api", auth: { type: "bearer" } },
                  },
                ],
              },
            },
          },
        ],
      },
    },
  ];
  mockSubmit.mockResolvedValue({
    status: "active",
    resolved: [{ tool_call_id: "tool_approval_call", outcome: "allow" }],
  });
});

it("makes pending credential consent actionable on the session approvals page", async () => {
  render(<SessionApprovalsPage />);
  expect(screen.getByText("Save credentials for this service?")).toBeInTheDocument();
  expect(screen.getByText("Alice")).toBeInTheDocument();
  expect(screen.getByText("https://mail.example/api")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /always/i })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Approve capture" }));
  await waitFor(() =>
    expect(mockSubmit).toHaveBeenCalledWith("session_test", [
      { tool_call_id: "tool_approval_call", decision: "allow" },
    ]),
  );
});

it("does not offer a stale consent card after the session resumes", () => {
  mockContext.events.push({
    id: "event_resume",
    session_id: mockContext.sessionId,
    type: "session.activated",
    ts: "2026-10-19T00:00:01Z",
    context: {},
    data: { turn_id: "turn_next" },
  });
  render(<SessionApprovalsPage />);
  expect(screen.queryByText("Save credentials for this service?")).not.toBeInTheDocument();
});
