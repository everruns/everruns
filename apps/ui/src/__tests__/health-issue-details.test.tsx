import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HealthIssueDetails } from "@/components/health/health-issue-details";
import type { HealthIssue } from "@/lib/api/types";
jest.mock("@/components/chat/continue-in-chat", () => ({
  ContinueInChat: () => <button>Continue in chat</button>,
}));
const check = jest.fn();
const snooze = jest.fn();
const runFixer = jest.fn();
const dismiss = jest.fn();
let mockDismissError: Error | null = null;
let mockManaged = false;
let mockAuthenticated = true;
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => ({
    requiresAuth: mockAuthenticated,
    user: mockAuthenticated ? { id: "user_test" } : null,
  }),
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_test" } }),
}));
jest.mock("@/hooks/use-health-issues", () => ({
  useHealthIssueAction: (action: string) => ({
    mutate: action === "check" ? check : snooze,
    isPending: false,
    error: null,
  }),
  useRunHealthFixer: () => ({ mutateAsync: runFixer, isPending: false }),
  useDismissFinding: () => ({ mutateAsync: dismiss, isPending: false, error: mockDismissError }),
}));
jest.mock("@/lib/api/agent-channels", () => ({
  getAgentChannel: jest.fn(() =>
    Promise.resolve({
      channel_config: {
        slack_app_provisioned: mockManaged,
        ...(mockManaged ? { slack_app_id: "A0123" } : {}),
        team_id: "T1",
      },
    }),
  ),
}));
const issue: HealthIssue = {
  episode_id: "00000000-0000-7000-8000-000000000002",
  id: "00000000-0000-7000-8000-000000000001",
  code: "slack.permissions",
  agent_id: "agent_1",
  agent_name: "Support",
  channel_id: "ep_1",
  status: "open",
  title: "Slack reactions need an additional permission",
  body: "This bot cannot add emoji reactions.",
  missing_scopes: ["reactions:write"],
  error_code: "missing_scope",
  stale: false,
  first_detected_at: "2026-10-03T00:00:00Z",
  last_checked_at: "2026-10-03T00:00:00Z",
  notification_id: null,
  snoozed_until: null,
  href: "/settings/health?issue=1",
  fixers: [],
};
const fixer = {
  kind: "slack.add_permissions",
  label: "Fix in Slack",
  steps: ["Everruns adds reactions:write to the Support app in Slack."],
  effect: "consent",
  undoable: false,
  needs_provider_admin: true,
  unavailable_reason: null,
  unblock: null,
};
function mount(value = issue, canManage = true) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <HealthIssueDetails issue={value} canManage={canManage} />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  jest.clearAllMocks();
  mockManaged = false;
  mockAuthenticated = true;
  mockDismissError = null;
});
test("manual repair explains reinstall and token update; check and snooze remain distinct", async () => {
  mount();
  expect(await screen.findByRole("link", { name: "Open integration settings" })).toHaveAttribute(
    "href",
    "/agents/agent_1/channels/ep_1",
  );
  expect(screen.getByText(/Reinstall to Workspace/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "integration settings" })).toHaveAttribute(
    "href",
    "/agents/agent_1/channels/ep_1",
  );
  fireEvent.click(screen.getByRole("button", { name: "Check again" }));
  expect(check).toHaveBeenCalledWith(issue.id);
  fireEvent.click(screen.getByRole("button", { name: "Remind me tomorrow" }));
  expect(snooze).toHaveBeenCalledWith(issue.id);
});
test("a declared fixer explains its steps and runs by kind", async () => {
  mockManaged = true;
  runFixer.mockResolvedValue({
    status: "consent",
    consent_url: "https://evil.example/oauth",
    returns_automatically: true,
    issue,
  });
  mount({ ...issue, fixers: [fixer] });
  expect(screen.getByText(/adds reactions:write to the Support app/)).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Open integration settings" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Fix in Slack" }));
  expect(runFixer).toHaveBeenCalledWith({ id: issue.id, kind: "slack.add_permissions" });
  // A consent URL outside the provider allowlist is never followed.
  expect(await screen.findByRole("alert")).toHaveTextContent(/unexpected approval page/);
});
test("a fixer nothing in Everruns unblocks says why and keeps the manual steps", async () => {
  mount({
    ...issue,
    fixers: [{ ...fixer, unavailable_reason: "This deployment cannot edit Slack apps." }],
  });
  expect(screen.getByText("Fix not available here")).toBeInTheDocument();
  expect(screen.getByText("This deployment cannot edit Slack apps.")).toBeInTheDocument();
  // No greyed-out fix button that leads nowhere.
  expect(screen.queryByRole("button", { name: "Fix in Slack" })).not.toBeInTheDocument();
  expect(await screen.findByText(/Reinstall to Workspace/)).toBeInTheDocument();
});
test("a blocked fixer links to the step that unblocks it", async () => {
  mockManaged = true;
  mount({
    ...issue,
    missing_scopes: ["reactions:read"],
    fixers: [
      {
        ...fixer,
        unavailable_reason: "Reconnect the workspace, then come back and run the fix.",
        unblock: { label: "Reconnect Slack workspace", href: "/settings/slack" },
      },
    ],
  });
  expect(screen.getByText("One step before the fix")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Reconnect Slack workspace" })).toHaveAttribute(
    "href",
    "/settings/slack",
  );
  expect(screen.queryByRole("button", { name: "Fix in Slack" })).not.toBeInTheDocument();
  // Everruns-made apps get the manual steps too, straight to the app's page.
  expect(await screen.findByRole("link", { name: "Open the app in Slack" })).toHaveAttribute(
    "href",
    "https://api.slack.com/apps/A0123/oauth",
  );
  expect(screen.getByText(/nothing to paste back/)).toBeInTheDocument();
});
test("an unblock link to another origin is not followed", () => {
  mount({
    ...issue,
    fixers: [
      {
        ...fixer,
        unavailable_reason: "Blocked.",
        unblock: { label: "Go", href: "//evil.example/x" },
      },
    ],
  });
  expect(screen.queryByRole("link", { name: "Go" })).not.toBeInTheDocument();
});
test("viewers see escalation and cannot reconnect", () => {
  mount(issue, false);
  expect(screen.getByText(/administrator needs to reconnect/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
});
test("stale evidence remains open and resolved evidence removes repair actions", () => {
  const rendered = mount({ ...issue, stale: true });
  expect(screen.getByText(/issue remains open until a fresh check/)).toBeInTheDocument();
  rendered.unmount();
  mount({ ...issue, status: "resolved", title: "Slack permissions verified" });
  expect(screen.queryByRole("button", { name: "Remind me tomorrow" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
});

test("anonymous local users can inspect health without personal reminder actions", () => {
  mockAuthenticated = false;
  mount();
  expect(screen.getByRole("button", { name: "Check again" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Remind me tomorrow" })).not.toBeInTheDocument();
});

test("outdated recovery evidence can be checked without claiming the issue is open", () => {
  mount({ ...issue, status: "resolved", stale: true, title: "Slack permissions verified" });
  expect(
    screen.getByText(/fresh check before relying on this previous result/),
  ).toBeInTheDocument();
  expect(screen.queryByText(/issue remains open until/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check again" }));
  expect(check).toHaveBeenCalledWith(issue.id);
  expect(screen.queryByRole("button", { name: "Remind me tomorrow" })).not.toBeInTheDocument();
});

test("organization issue has no Slack repair and does not load a channel", () => {
  const { getAgentChannel } = jest.requireMock("@/lib/api/agent-channels");
  mount({
    ...issue,
    code: "org.active_turn_limit",
    agent_id: null,
    agent_name: null,
    channel_id: null,
    missing_scopes: [],
    error_code: null,
    title: "Active turn limit reached",
    body: "New messages are refused while this organization has 1000 turns running at once.",
  });
  expect(screen.getByText("Organization")).toBeInTheDocument();
  expect(screen.queryByText(/Review and approve the permissions in Slack/)).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Open integration settings" })).not.toBeInTheDocument();
  expect(getAgentChannel).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Check again" }));
  expect(check).toHaveBeenCalledWith(issue.id);
  fireEvent.click(screen.getByRole("button", { name: "Remind me tomorrow" }));
  expect(snooze).toHaveBeenCalledWith(issue.id);
});

const spendIssue: HealthIssue = {
  ...issue,
  code: "agent.unattended_spend_spike",
  channel_id: null,
  subject_id: "agent_1",
  missing_scopes: [],
  title: "Unattended spend jumped",
  evidence: {
    last_day_usd: 6.61,
    baseline_daily_usd: 0.31,
    window_end: "2026-11-25T19:00:00Z",
    top_sessions: [{ session_id: "session_01a20fbf2bd4741f85c2ea934b262895", amount_usd: 5.5 }],
  },
};

test("spend alerts explain their baseline and link to contributing sessions and agent controls", () => {
  mount(spendIssue);
  const breakdown = screen.getByRole("region", { name: "Spend breakdown" });
  expect(within(breakdown).getByText("$6.61")).toBeInTheDocument();
  expect(within(breakdown).getByText("$0.31")).toBeInTheDocument();
  expect(within(breakdown).getByText("+$6.30")).toBeInTheDocument();
  expect(within(breakdown).getByText(/six days before/)).toBeInTheDocument();
  expect(within(breakdown).getByText(/does not mean/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Session 4b262895/ })).toHaveAttribute(
    "href",
    "/sessions/session_01a20fbf2bd4741f85c2ea934b262895/cost",
  );
  expect(screen.getByRole("link", { name: "Agent sessions" })).toHaveAttribute(
    "href",
    "/agents/agent_1?tab=sessions",
  );
  expect(screen.getByRole("link", { name: "Channels and triggers" })).toHaveAttribute(
    "href",
    "/agents/agent_1?tab=integrations",
  );
});

test("dismissing an advisory alert requires a reason and retains errors for retry", async () => {
  dismiss.mockRejectedValue(new Error("Could not dismiss"));
  mockDismissError = new Error("Could not dismiss");
  mount(spendIssue);
  fireEvent.click(screen.getByRole("button", { name: "Dismiss alert" }));
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByRole("button", { name: "Dismiss" })).toBeDisabled();
  fireEvent.change(within(dialog).getByRole("textbox", { name: "Reason" }), {
    target: { value: "Expected scheduled audit" },
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Dismiss" }));
  await waitFor(() =>
    expect(dismiss).toHaveBeenCalledWith({ id: spendIssue.id, reason: "Expected scheduled audit" }),
  );
  expect(within(dialog).getByRole("alert")).toHaveTextContent("Could not dismiss");
  expect(screen.getByRole("dialog")).toBeInTheDocument();
});

test("dismissal is unavailable to viewers, closed alerts, and issues that require repair", () => {
  const view = mount(spendIssue, false);
  expect(screen.queryByRole("button", { name: "Dismiss alert" })).not.toBeInTheDocument();
  view.unmount();
  const closed = mount({ ...spendIssue, status: "resolved" });
  expect(screen.queryByRole("button", { name: "Dismiss alert" })).not.toBeInTheDocument();
  closed.unmount();
  mount(issue);
  expect(screen.queryByRole("button", { name: "Dismiss alert" })).not.toBeInTheDocument();
});

test("legacy and malformed spend evidence stays readable without unsafe links or invented ratios", () => {
  mount({
    ...spendIssue,
    evidence: {
      last_day_usd: 8,
      baseline_daily_usd: 0,
      top_sessions: [
        { session_id: "../../settings", amount_usd: 8 },
        { session_id: "session_01a20fbf2bd4741f85c2ea934b262895", amount_usd: -1 },
      ],
    },
  });
  expect(screen.queryByText(/Infinity/)).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: /Session / })).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Agent sessions" })).toBeInTheDocument();
});
