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
    name: "knowledge",
    label: "Knowledge",
    description: "Search documents",
    grade: "prod",
    effective: true,
    can_manage: true,
  },
  {
    name: "voice",
    label: "Voice",
    description: "Talk to agents",
    grade: "internal",
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

  it("shows adoption off and prod on without exposing internal enrolment", () => {
    render(<FeaturesSettingsPage />);
    expect(screen.getByRole("switch", { name: "Enable Evals" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.getByRole("switch", { name: "Enable Knowledge" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.queryByText("Voice")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Enable Knowledge" }));
    expect(mockMutate).toHaveBeenCalledWith({ knowledge: false });
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
    expect(screen.getByRole("switch", { name: "Enable Knowledge" })).toBeDisabled();
    fireEvent.click(screen.getByRole("switch", { name: "Enable Knowledge" }));
    expect(mockMutate).not.toHaveBeenCalled();
  });
});
