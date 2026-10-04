import { fireEvent, render, screen } from "@testing-library/react";
import FeaturesSettingsPage from "@/app/(main)/settings/features/page";

const mockMutate = jest.fn();
let mockRole = "owner";
const mockRows = [
  {
    name: "evals",
    label: "Evals",
    description: "Test agents",
    grade: "adoption",
    effective: false,
    can_manage: true,
  },
  {
    name: "reports",
    label: "Reports",
    description: "Explore usage",
    grade: "prod",
    effective: true,
    can_manage: true,
  },
  {
    name: "voice",
    label: "Voice",
    description: "Talk to agents",
    grade: "preview",
    effective: false,
    can_manage: false,
  },
];

jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { role: mockRole, name: "Test Org" } }),
}));
jest.mock("@/hooks/use-org-feature-flags", () => ({
  useOrgFeatureFlagSettings: () => ({ data: { flags: mockRows }, isLoading: false }),
  useUpdateOrgFeatureFlags: () => ({ mutate: mockMutate, isPending: false }),
}));

describe("Feature grade settings", () => {
  beforeEach(() => {
    mockRole = "owner";
    mockMutate.mockClear();
  });

  it("shows adoption off and prod on without exposing preview enrolment", () => {
    render(<FeaturesSettingsPage />);
    expect(screen.getByRole("switch", { name: "Enable Evals" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.getByRole("switch", { name: "Enable Reports" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.queryByText("Voice")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Enable Reports" }));
    expect(mockMutate).toHaveBeenCalledWith({ reports: false });
  });

  it("lets an org admin opt into adoption", () => {
    mockRole = "admin";
    render(<FeaturesSettingsPage />);
    fireEvent.click(screen.getByRole("switch", { name: "Enable Evals" }));
    expect(mockMutate).toHaveBeenCalledWith({ evals: true });
  });

  it("keeps member controls read only", () => {
    mockRole = "member";
    render(<FeaturesSettingsPage />);
    expect(screen.getByRole("switch", { name: "Enable Reports" })).toBeDisabled();
    fireEvent.click(screen.getByRole("switch", { name: "Enable Reports" }));
    expect(mockMutate).not.toHaveBeenCalled();
  });
});
