import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import {
  ZeroOrgOnboarding,
  type UseZeroOrgPolicy,
} from "@/components/onboarding/zero-org-onboarding";

// Mock next/navigation
const mockPush = jest.fn();
const mockReplace = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush, replace: mockReplace }),
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

const mockAcceptMutateAsync = jest.fn();
const pendingInvitationsState = {
  data: [] as Array<{
    id: string;
    org_id: string;
    org_name: string;
    role: "owner" | "admin" | "member";
    expires_at: string;
  }>,
  isLoading: false,
  isError: false,
};
const acceptInvitationState = {
  mutateAsync: mockAcceptMutateAsync,
  isPending: false,
  isError: false,
  variables: undefined as string | undefined,
};
jest.mock("@/hooks/use-invitations", () => ({
  usePendingInvitations: () => pendingInvitationsState,
  useAcceptPendingInvitation: () => acceptInvitationState,
}));

describe("ZeroOrgOnboarding", () => {
  beforeEach(() => {
    mockPush.mockClear();
    mockReplace.mockClear();
    mockMutateAsync.mockClear();
    mockAcceptMutateAsync.mockClear();
    mockSetCurrentOrg.mockClear();
    createOrgState.isPending = false;
    createOrgState.isError = false;
    createOrgState.error = null;
    pendingInvitationsState.data = [];
    pendingInvitationsState.isLoading = false;
    pendingInvitationsState.isError = false;
    acceptInvitationState.isPending = false;
    acceptInvitationState.isError = false;
    acceptInvitationState.variables = undefined;
  });

  it("offers pending invitations before organization creation", () => {
    pendingInvitationsState.data = [
      {
        id: "orginv_one",
        org_id: "org_acme",
        org_name: "Acme",
        role: "member",
        expires_at: "2026-09-22T00:00:00Z",
      },
      {
        id: "orginv_two",
        org_id: "org_beta",
        org_name: "Beta Labs",
        role: "admin",
        expires_at: "2026-09-22T00:00:00Z",
      },
    ];

    render(<ZeroOrgOnboarding />);

    expect(screen.getByRole("heading", { name: "Join your team" })).toBeInTheDocument();
    expect(screen.getByText("Acme")).toBeInTheDocument();
    expect(screen.getByText("Beta Labs")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Join" })).toHaveLength(2);
    expect(screen.getByLabelText("Organisation name")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /create organisation/i })).toBeInTheDocument();
  });

  it("joins a pending invitation and continues to chats", async () => {
    pendingInvitationsState.data = [
      {
        id: "orginv_one",
        org_id: "org_acme",
        org_name: "Acme",
        role: "member",
        expires_at: "2026-09-22T00:00:00Z",
      },
    ];
    mockAcceptMutateAsync.mockResolvedValue({ org_id: "org_acme", role: "member" });

    render(<ZeroOrgOnboarding />);
    fireEvent.click(screen.getByRole("button", { name: "Join" }));

    await waitFor(() => {
      expect(mockAcceptMutateAsync).toHaveBeenCalledWith("orginv_one");
      expect(mockReplace).toHaveBeenCalledWith("/chats");
    });
  });

  it("keeps organization creation available when invitation loading fails", () => {
    pendingInvitationsState.isError = true;

    render(<ZeroOrgOnboarding />);

    expect(screen.getByRole("alert")).toHaveTextContent(/invitations could not be loaded/i);
    expect(screen.getByRole("heading", { name: "Create your organisation" })).toBeInTheDocument();
    expect(screen.getByLabelText("Organisation name")).toBeInTheDocument();
  });

  it("shows a generic join error without navigating", () => {
    pendingInvitationsState.data = [
      {
        id: "orginv_one",
        org_id: "org_acme",
        org_name: "Acme",
        role: "member",
        expires_at: "2026-09-22T00:00:00Z",
      },
    ];
    acceptInvitationState.isError = true;

    render(<ZeroOrgOnboarding />);

    expect(screen.getByRole("alert")).toHaveTextContent(/failed to join organisation/i);
    expect(mockReplace).not.toHaveBeenCalled();
  });

  it("creates the first org and redirects to setup (default OSS flow)", async () => {
    mockMutateAsync.mockResolvedValue({ id: "org_new123", name: "Acme" });

    render(<ZeroOrgOnboarding />);

    const input = screen.getByLabelText("Organisation name");
    fireEvent.change(input, { target: { value: "Acme" } });
    fireEvent.click(screen.getByRole("button", { name: /create organisation/i }));

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

    const submit = screen.getByRole("button", { name: /create organisation/i });
    expect(submit).toBeDisabled();
    fireEvent.click(submit);
    expect(mockMutateAsync).not.toHaveBeenCalled();
  });

  it("surfaces a create error generically (no raw server message)", () => {
    createOrgState.isError = true;
    createOrgState.error = new Error("boom");

    render(<ZeroOrgOnboarding />);

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(/failed to create organisation/i);
    // Server error strings must never render (TM-AUTH-019 discipline).
    expect(alert).not.toHaveTextContent(/boom/);
  });

  it("swallows a create rejection without redirecting (no unhandled rejection)", async () => {
    mockMutateAsync.mockRejectedValue(new Error("boom"));

    render(<ZeroOrgOnboarding />);

    fireEvent.change(screen.getByLabelText("Organisation name"), { target: { value: "Acme" } });
    fireEvent.click(screen.getByRole("button", { name: /create organisation/i }));

    await waitFor(() => expect(mockMutateAsync).toHaveBeenCalledWith({ name: "Acme" }));
    expect(mockSetCurrentOrg).not.toHaveBeenCalled();
    expect(mockPush).not.toHaveBeenCalled();
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
    expect(screen.queryByLabelText("Organisation name")).not.toBeInTheDocument();
  });

  it("renders a loading state from the policy hook", () => {
    const usePolicy: UseZeroOrgPolicy = () => ({ status: "loading" });

    render(<ZeroOrgOnboarding usePolicy={usePolicy} />);

    expect(screen.queryByLabelText("Organisation name")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /create organisation/i })).not.toBeInTheDocument();
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

    fireEvent.change(screen.getByLabelText("Organisation name"), {
      target: { value: "SaaS Co" },
    });
    fireEvent.click(screen.getByRole("button", { name: /create organisation/i }));

    await waitFor(() => {
      expect(overrideMutate).toHaveBeenCalledWith({ name: "SaaS Co" });
      expect(mockMutateAsync).not.toHaveBeenCalled();
      expect(mockPush).toHaveBeenCalledWith("/orgs/org_saas/setup");
    });
  });
});
