import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HealthIssueDetails } from "@/components/health/health-issue-details";
import type { HealthIssue } from "@/lib/api/types";
const check = jest.fn();
const snooze = jest.fn();
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
}));
jest.mock("@/lib/api/agent-channels", () => ({
  getAgentChannel: jest.fn(() =>
    Promise.resolve({ channel_config: { slack_app_provisioned: mockManaged, team_id: "T1" } }),
  ),
  beginSlackInstall: jest.fn(),
}));
const issue: HealthIssue = {
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
});
test("manual repair explains reinstall and token update; check and snooze remain distinct", async () => {
  mount();
  expect(await screen.findByRole("link", { name: "Open integration settings" })).toHaveAttribute(
    "href",
    "/agents/agent_1/channels/ep_1",
  );
  expect(screen.getByText(/reinstall the existing app/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check again" }));
  expect(check).toHaveBeenCalledWith(issue.id);
  fireEvent.click(screen.getByRole("button", { name: "Remind me tomorrow" }));
  expect(snooze).toHaveBeenCalledWith(issue.id);
});
test("managed installation offers Slack consent", async () => {
  mockManaged = true;
  mount();
  expect(await screen.findByRole("button", { name: "Reconnect Slack" })).toBeInTheDocument();
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
