import { fireEvent, render, screen, within } from "@testing-library/react";
import HealthPage from "@/app/(main)/settings/health/page";

let mockSelected = "first";
let mockTotal = 2;
let mockCanView = true;
const mockList = jest.fn();
const first = {
  id: "first",
  title: "Spend increased",
  agent_name: "Housekeeping",
  status: "open",
  stale: false,
};
const second = { ...first, id: "second", title: "Sign in again" };
jest.mock("next/navigation", () => ({
  useSearchParams: () => new URLSearchParams(mockSelected ? { issue: mockSelected } : {}),
}));
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, ...props }: React.ComponentProps<"a">) => <a {...props}>{children}</a>,
}));
jest.mock("@/hooks", () => ({ usePageTitle: () => undefined }));
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => mockCanView, isLoading: false }),
}));
jest.mock("@/hooks/use-health-issues", () => ({
  useHealthIssues: (...args: unknown[]) => {
    mockList(...args);
    return { data: { data: [first, second], total: mockTotal, limit: 20 }, isLoading: false };
  },
  useHealthIssue: (id: string | null) => ({
    data: id ? { ...first, id } : undefined,
    isLoading: false,
  }),
}));
jest.mock("@/components/chat-mode/page-view-state", () => ({
  useChatPageRows: () => undefined,
  useChatPageFilters: () => undefined,
}));
jest.mock("@/providers/notifications-provider", () => ({
  useOptionalNotificationsContext: () => null,
}));
jest.mock("@/components/health/health-fix-queue", () => ({ HealthFixQueue: () => null }));
jest.mock("@/components/health/health-issue-details", () => ({
  HealthIssueDetails: ({ issue }: { issue: { id: string } }) => (
    <section aria-label="Details">{issue.id}</section>
  ),
}));

beforeEach(() => {
  mockSelected = "first";
  mockTotal = 2;
  mockCanView = true;
  mockList.mockClear();
});

test("issues precede their details, identify the selected row, and omit one-page controls", () => {
  render(<HealthPage />);
  const list = screen.getByRole("navigation", { name: "Health issues" });
  expect(within(list).getByRole("link", { name: /Spend increased/ })).toHaveAttribute(
    "aria-current",
    "true",
  );
  expect(within(list).getByRole("link", { name: /Sign in again/ })).toHaveAttribute(
    "href",
    "/settings/health?issue=second",
  );
  expect(
    list.compareDocumentPosition(screen.getByRole("region", { name: "Details" })) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Previous" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Next" })).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Back to issues" })).toHaveAttribute(
    "href",
    "/settings/health",
  );
});

test("real pagination shows the range, advances, and returns to the first page", () => {
  mockTotal = 25;
  render(<HealthPage />);
  expect(screen.getByText("1–20 of 25")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Previous" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(mockList).toHaveBeenLastCalledWith(undefined, 20, true);
  expect(screen.getByText("21–25 of 25")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Previous" }));
  expect(mockList).toHaveBeenLastCalledWith(undefined, 0, true);
});

test("an unselected list explains selection and permissions still guard the screen", () => {
  mockSelected = "";
  const view = render(<HealthPage />);
  expect(screen.getByText("Select an issue to see its details and actions.")).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Details" })).not.toBeInTheDocument();
  view.unmount();
  mockCanView = false;
  render(<HealthPage />);
  expect(screen.queryByRole("navigation", { name: "Health issues" })).not.toBeInTheDocument();
  expect(mockList).toHaveBeenLastCalledWith(undefined, 0, false);
});

test("closing the final item on a later page returns to a valid range", () => {
  mockTotal = 21;
  const view = render(<HealthPage />);
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(mockList).toHaveBeenLastCalledWith(undefined, 20, true);
  mockTotal = 20;
  view.rerender(<HealthPage />);
  expect(mockList).toHaveBeenLastCalledWith(undefined, 0, true);
  expect(screen.queryByRole("button", { name: "Next" })).not.toBeInTheDocument();
});
