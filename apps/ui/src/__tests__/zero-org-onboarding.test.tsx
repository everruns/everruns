import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import {
  ZeroOrgOnboarding,
  type UseZeroOrgPolicy,
} from "@/components/onboarding/zero-org-onboarding";

// Mock next/navigation
const mockPush = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush, replace: jest.fn() }),
}));

// Mock org provider
const mockSetCurrentOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ setCurrentOrg: mockSetCurrentOrg }),
}));

// Mock the default create-organization hook
const mockMutateAsync = jest.fn();
const createOrgState = {
  mutateAsync: mockMutateAsync,
  isPending: false,
  isError: false,
  error: null as Error | null,
};
jest.mock("@/hooks/use-organizations", () => ({
  useCreateOrganization: () => createOrgState,
}));

describe("ZeroOrgOnboarding", () => {
  beforeEach(() => {
    mockPush.mockClear();
    mockMutateAsync.mockClear();
    mockSetCurrentOrg.mockClear();
    createOrgState.isPending = false;
    createOrgState.isError = false;
    createOrgState.error = null;
  });

  it("creates the first org and redirects to setup (default OSS flow)", async () => {
    mockMutateAsync.mockResolvedValue({ id: "org_new123", name: "Acme" });

    render(<ZeroOrgOnboarding />);

    const input = screen.getByLabelText("Organization name");
    fireEvent.change(input, { target: { value: "Acme" } });
    fireEvent.click(screen.getByRole("button", { name: /create organization/i }));

    await waitFor(() => {
      expect(mockMutateAsync).toHaveBeenCalledWith({ name: "Acme" });
      expect(mockSetCurrentOrg).toHaveBeenCalledWith({
        public_id: "org_new123",
        name: "Acme",
        role: "owner",
      });
      expect(mockPush).toHaveBeenCalledWith("/orgs/org_new123/setup");
    });
  });

  it("does not submit when the name is blank", () => {
    render(<ZeroOrgOnboarding />);

    const submit = screen.getByRole("button", { name: /create organization/i });
    expect(submit).toBeDisabled();
    fireEvent.click(submit);
    expect(mockMutateAsync).not.toHaveBeenCalled();
  });

  it("surfaces a create error", () => {
    createOrgState.isError = true;
    createOrgState.error = new Error("boom");

    render(<ZeroOrgOnboarding />);

    expect(screen.getByText(/failed to create organization: boom/i)).toBeInTheDocument();
  });

  it("renders a wrapper policy-blocked gate instead of the form", () => {
    const usePolicy: UseZeroOrgPolicy = () => ({
      status: "blocked",
      title: "Verify your email",
      body: "Confirm your email before creating an organization.",
      actions: <button type="button">Resend email</button>,
    });

    render(<ZeroOrgOnboarding usePolicy={usePolicy} />);

    expect(screen.getByText("Verify your email")).toBeInTheDocument();
    expect(
      screen.getByText("Confirm your email before creating an organization."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Resend email" })).toBeInTheDocument();
    // Form is hidden while blocked
    expect(screen.queryByLabelText("Organization name")).not.toBeInTheDocument();
  });

  it("renders a loading state from the policy hook", () => {
    const usePolicy: UseZeroOrgPolicy = () => ({ status: "loading" });

    render(<ZeroOrgOnboarding usePolicy={usePolicy} />);

    expect(screen.queryByLabelText("Organization name")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /create organization/i })).not.toBeInTheDocument();
  });

  it("honors a wrapper create-org override", async () => {
    const overrideMutate = jest.fn().mockResolvedValue({ id: "org_saas", name: "SaaS Co" });
    const useCreateOrg = (() => ({
      mutateAsync: overrideMutate,
      isPending: false,
      isError: false,
      error: null,
    })) as unknown as typeof import("@/hooks/use-organizations").useCreateOrganization;

    render(<ZeroOrgOnboarding useCreateOrg={useCreateOrg} />);

    fireEvent.change(screen.getByLabelText("Organization name"), {
      target: { value: "SaaS Co" },
    });
    fireEvent.click(screen.getByRole("button", { name: /create organization/i }));

    await waitFor(() => {
      expect(overrideMutate).toHaveBeenCalledWith({ name: "SaaS Co" });
      expect(mockMutateAsync).not.toHaveBeenCalled();
      expect(mockPush).toHaveBeenCalledWith("/orgs/org_saas/setup");
    });
  });
});
