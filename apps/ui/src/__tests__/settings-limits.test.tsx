import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import LimitsSettingsPage from "@/app/(main)/settings/limits/page";
import { parseLimitsForm } from "@/components/organizations/background-limits-settings";
import { budgetSubjectLabel, DEFAULT_BACKGROUND_LIMITS } from "@/lib/background-limits";
import { settingsNavigationSections } from "@/lib/settings-navigation";
import type { BackgroundLimits } from "@/lib/api/types";

let mockOrganization: { id: string; background_limits?: BackgroundLimits } = { id: "org-1" };
let mockCanManage = true;
const mockMutate = jest.fn();
jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));
jest.mock("@/components/chat-mode/use-page-read-context", () => ({
  usePageReadContext: jest.fn(),
}));
jest.mock("@/providers/org-provider", () => ({ useOrg: () => ({ hasRole: () => mockCanManage }) }));
jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({ data: mockOrganization, isLoading: false, error: null }),
  useUpdateOrganization: () => ({ mutateAsync: mockMutate, isPending: false }),
}));

beforeEach(() => {
  mockOrganization = {
    id: "org-1",
    background_limits: {
      schedule_max_lifetime_days: 3,
      schedule_max_runs: 48,
      schedule_budget_usd: 2,
      schedule_max_budget_usd: 10,
      org_unattended_daily_usd: 20,
    },
  };
  mockCanManage = true;
  mockMutate.mockReset().mockImplementation(async (patch) => ({ ...mockOrganization, ...patch }));
});

test("shows the stored limits with the platform ceilings as input maximums", () => {
  render(<LimitsSettingsPage />);
  const days = screen.getByLabelText("Longest an agent's schedule may run");
  expect(days).toHaveValue(3);
  expect(days).toHaveAttribute("max", "7");
  const runs = screen.getByLabelText("Most runs per agent schedule");
  expect(runs).toHaveValue(48);
  expect(runs).toHaveAttribute("max", "96");
  expect(screen.getByLabelText("Default budget per schedule")).toHaveValue(2);
  expect(screen.getByLabelText("Largest budget an agent may give a schedule")).toHaveValue(10);
  expect(
    screen.getByLabelText("Daily cap on unattended work (schedules, triggers, retries)"),
  ).toHaveValue(20);
  expect(screen.getByRole("switch", { name: "No cap" })).toHaveAttribute("aria-checked", "false");
});

test("saves the whole object, with null for no daily cap", async () => {
  render(<LimitsSettingsPage />);
  fireEvent.change(screen.getByLabelText("Most runs per agent schedule"), {
    target: { value: "24" },
  });
  fireEvent.click(screen.getByRole("switch", { name: "No cap" }));
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mockMutate).toHaveBeenCalledWith({
      background_limits: {
        schedule_max_lifetime_days: 3,
        schedule_max_runs: 24,
        schedule_budget_usd: 2,
        schedule_max_budget_usd: 10,
        org_unattended_daily_usd: null,
      },
    }),
  );
});

test("rejects values past the ceilings without calling the API", () => {
  render(<LimitsSettingsPage />);
  fireEvent.change(screen.getByLabelText("Longest an agent's schedule may run"), {
    target: { value: "8" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(screen.getByRole("alert")).toHaveTextContent("from 1 to 7");
  expect(mockMutate).not.toHaveBeenCalled();
});

test("members see the limits read-only", () => {
  mockCanManage = false;
  render(<LimitsSettingsPage />);
  expect(screen.getByLabelText("Most runs per agent schedule")).toHaveAttribute("readonly");
  expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
  expect(screen.getByText(/Only organization owners and admins/)).toBeInTheDocument();
});

test("an older server without background_limits shows the code defaults", () => {
  mockOrganization = { id: "org-1" };
  render(<LimitsSettingsPage />);
  expect(screen.getByLabelText("Most runs per agent schedule")).toHaveValue(
    DEFAULT_BACKGROUND_LIMITS.schedule_max_runs,
  );
});

test("parseLimitsForm keeps the default budget within the largest budget", () => {
  expect(
    parseLimitsForm({
      schedule_max_lifetime_days: "7",
      schedule_max_runs: "96",
      schedule_budget_usd: "12",
      schedule_max_budget_usd: "10",
      org_unattended_daily_usd: "20",
      noDailyCap: false,
    }),
  ).toBe("Default budget per schedule cannot be more than the largest budget.");
});

test("Limits is in the organization settings navigation", () => {
  const items = settingsNavigationSections.flatMap((section) => section.items);
  expect(items.find((item) => item.href === "/settings/limits")?.name).toBe("Limits");
});

test("labels the background budget subjects", () => {
  expect(budgetSubjectLabel("session_schedule")).toBe("Schedule");
  expect(budgetSubjectLabel("org_unattended")).toBe("Unattended work (daily)");
  expect(budgetSubjectLabel("something_new")).toBe("something_new");
});
