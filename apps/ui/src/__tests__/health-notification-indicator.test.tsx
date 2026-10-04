import { fireEvent, render, screen } from "@testing-library/react";
import { NotificationBell, NotificationIndicator } from "@/components/layout/notification-bell";

let mockUnviewedCount = 0;
let mockUnresolved = 1;
let mockEnabled = true;
const mockNotice = {
  id: "notice-1",
  kind: "health.issue",
  title: "Slack permissions need attention",
  body: "Reconnect Slack",
  target_type: "health_issue",
  href: "/settings/health?issue=issue-1",
  viewed_at: "2026-10-03T00:00:00Z",
  created_at: "2026-10-03T00:00:00Z",
  occurrence_count: 1,
  payload: {},
};
let mockNotices = [mockNotice];
jest.mock("next/navigation", () => ({ useRouter: () => ({ push: jest.fn() }) }));
jest.mock("@/providers/notifications-provider", () => ({
  useOptionalNotificationsContext: () => ({
    isEnabled: mockEnabled,
    unviewedCount: mockUnviewedCount,
  }),
  useNotificationsContext: () => ({
    isEnabled: mockEnabled,
    unviewedCount: mockUnviewedCount,
    notifications: mockNotices,
    openNotification: jest.fn(),
    markViewed: jest.fn(),
  }),
}));
jest.mock("@/hooks/use-health-issues", () => ({
  useHealthIssues: () => ({
    data: {
      total: mockUnresolved,
      data: [
        {
          id: "issue-1",
          title: "Slack permissions need attention",
          body: "Reconnect Slack",
          agent_name: "Support",
          stale: false,
          href: "/settings/health?issue=issue-1",
        },
      ],
    },
  }),
}));

beforeEach(() => {
  mockUnviewedCount = 0;
  mockUnresolved = 1;
  mockEnabled = true;
  mockNotices = [mockNotice];
});

test("reading the announcement leaves a pending health indicator", () => {
  render(<NotificationIndicator />);
  expect(screen.getByText("0 unread notifications; 1 unresolved issues")).toBeInTheDocument();
});

test("fresh recovery removes the indicator when no unread activity remains", () => {
  mockUnresolved = 0;
  const { container } = render(<NotificationIndicator />);
  expect(container).toBeEmptyDOMElement();
});

test("regular unread activity keeps its indicator after recovery", () => {
  mockUnresolved = 0;
  mockUnviewedCount = 2;
  render(<NotificationIndicator />);
  expect(screen.getByText("2 unread notifications; 0 unresolved issues")).toBeInTheDocument();
});

test("disabled notifications hide the menu indicator", () => {
  mockEnabled = false;
  const { container } = render(<NotificationIndicator />);
  expect(container).toBeEmptyDOMElement();
});

test("a read health announcement stays actionable without a session shortcut", async () => {
  render(<NotificationBell />);
  fireEvent.click(
    screen.getByRole("button", { name: "Notifications, 0 unread, 1 unresolved issues" }),
  );
  expect(
    await screen.findByRole("menuitem", { name: /Slack permissions need attention/ }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: "Open sessions" })).not.toBeInTheDocument();
});

test("session activity preserves the session shortcut", async () => {
  mockNotices = [
    {
      ...mockNotice,
      kind: "session.completed",
      target_type: "session",
      href: "/sessions/session-1/chat",
    },
  ];
  render(<NotificationBell />);
  fireEvent.click(screen.getByRole("button", { name: /Notifications/ }));
  expect(await screen.findByRole("menuitem", { name: "Open sessions" })).toBeInTheDocument();
});
