import { render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import MembersPage from "@/app/(main)/settings/members/page";

// Mock members data: an owner, an admin, and a plain member.
const mockMembers = [
  {
    user_id: "user-1",
    email: "owner@example.com",
    name: "Owner User",
    avatar_url: "https://example.com/avatar1.jpg",
    role: "owner" as const,
    joined_at: "2024-01-01T00:00:00Z",
  },
  {
    user_id: "user-3",
    email: "admin@example.com",
    name: "Admin User",
    avatar_url: null,
    role: "admin" as const,
    joined_at: "2024-02-01T00:00:00Z",
  },
  {
    user_id: "user-2",
    email: "member@example.com",
    name: "Regular Member",
    avatar_url: null,
    role: "member" as const,
    joined_at: "2024-02-15T00:00:00Z",
  },
];

const mockUseMembers = jest.fn();
const mockUseUpdateMemberRole = jest.fn();
const mockUseRemoveMember = jest.fn();
const mockUseAuth = jest.fn();
const mockUseOrg = jest.fn();

jest.mock("@/hooks/use-members", () => ({
  useMembers: () => mockUseMembers(),
  useUpdateMemberRole: () => mockUseUpdateMemberRole(),
  useRemoveMember: () => mockUseRemoveMember(),
}));

jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => mockUseAuth(),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => mockUseOrg(),
}));

const ROLE_LEVELS: Record<string, number> = { owner: 3, admin: 2, member: 1 };

/** Build a useOrg() mock whose hasRole mirrors the real role hierarchy. */
function orgMock(role: "owner" | "admin" | "member") {
  return {
    currentOrg: { public_id: "org-123", name: "Test Org", role },
    hasRole: (required: string) => (ROLE_LEVELS[role] ?? 0) >= (ROLE_LEVELS[required] ?? 0),
    isLoading: false,
  };
}

/** Locate the member card row containing the given display name. */
function cardFor(name: string): HTMLElement {
  return screen.getByText(name).closest(".border") as HTMLElement;
}

describe("MembersPage", () => {
  let queryClient: QueryClient;

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });

    // Default: viewer is the owner (user-1).
    mockUseAuth.mockReturnValue({
      user: { id: "user-1" },
      requiresAuth: true,
    });

    mockUseOrg.mockReturnValue(orgMock("owner"));

    mockUseMembers.mockReturnValue({
      data: mockMembers,
      isLoading: false,
      error: null,
    });

    mockUseUpdateMemberRole.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
      isError: false,
      error: null,
    });

    mockUseRemoveMember.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
      isError: false,
      error: null,
    });
  });

  it("renders Members section header", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Members")).toBeInTheDocument();
    expect(screen.getByText(/Manage members of/)).toBeInTheDocument();
  });

  it("renders member cards with correct data", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Owner User")).toBeInTheDocument();
    expect(screen.getByText("Regular Member")).toBeInTheDocument();
    expect(screen.getByText("owner@example.com")).toBeInTheDocument();
    expect(screen.getByText("member@example.com")).toBeInTheDocument();
  });

  it("shows role badge for current user (self)", () => {
    render(<MembersPage />, { wrapper });

    // Current user (user-1) should show its own role as a badge, not a dropdown.
    const ownerCard = cardFor("Owner User");
    expect(within(ownerCard).getByText("Owner")).toBeInTheDocument();
    expect(within(ownerCard).getByText("You")).toBeInTheDocument();
  });

  it("shows loading skeleton when loading", () => {
    mockUseMembers.mockReturnValue({
      data: [],
      isLoading: true,
      error: null,
    });

    render(<MembersPage />, { wrapper });

    const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
    expect(skeletons.length).toBeGreaterThan(0);
  });

  it("shows empty state when no members exist", () => {
    mockUseMembers.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<MembersPage />, { wrapper });

    expect(screen.getByText("No members")).toBeInTheDocument();
  });

  it("shows error message when API fails", () => {
    mockUseMembers.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<MembersPage />, { wrapper });

    expect(screen.getByText(/Failed to load members/)).toBeInTheDocument();
  });

  it("shows member count", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("3 members")).toBeInTheDocument();
  });

  it("shows joined date for members", () => {
    render(<MembersPage />, { wrapper });

    const joinedDates = screen.getAllByText(/Joined/);
    expect(joinedDates.length).toBe(3);
  });

  it("shows org name in subtitle", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Test Org")).toBeInTheDocument();
  });

  describe("owner viewer", () => {
    it("can change any member's role and offers the Owner option", () => {
      render(<MembersPage />, { wrapper });

      // Non-self admin + member cards render role dropdowns (2 comboboxes).
      const comboboxes = screen.getAllByRole("combobox");
      expect(comboboxes.length).toBe(2);
      // Owner promotion is available to an owner viewer.
      expect(screen.getAllByText("Owner").length).toBeGreaterThan(0);
    });

    it("shows remove buttons for other members", () => {
      render(<MembersPage />, { wrapper });

      // Owner may remove the admin and the member (not self).
      expect(screen.getAllByTitle("Remove member").length).toBe(2);
    });
  });

  describe("admin viewer", () => {
    beforeEach(() => {
      // Viewer is the admin (user-3).
      mockUseAuth.mockReturnValue({ user: { id: "user-3" }, requiresAuth: true });
      mockUseOrg.mockReturnValue(orgMock("admin"));
    });

    it("cannot demote the owner: shows a badge, not a dropdown", () => {
      render(<MembersPage />, { wrapper });

      const ownerCard = cardFor("Owner User");
      // Owner card renders a role badge, never a role combobox for an admin.
      expect(within(ownerCard).queryByRole("combobox")).not.toBeInTheDocument();
      expect(within(ownerCard).getByText("Owner")).toBeInTheDocument();
    });

    it("never shows any remove controls (owners-only)", () => {
      render(<MembersPage />, { wrapper });

      expect(screen.queryByTitle("Remove member")).not.toBeInTheDocument();
    });

    it("can re-role a non-owner member but is not offered the Owner option", () => {
      render(<MembersPage />, { wrapper });

      const memberCard = cardFor("Regular Member");
      const combobox = within(memberCard).getByRole("combobox");
      expect(combobox).toBeInTheDocument();
      // The Owner option must not be present for an admin viewer.
      expect(within(memberCard).queryByText("Owner")).not.toBeInTheDocument();
    });
  });

  describe("member viewer", () => {
    beforeEach(() => {
      mockUseAuth.mockReturnValue({ user: { id: "user-2" }, requiresAuth: true });
      mockUseOrg.mockReturnValue(orgMock("member"));
    });

    it("shows only badges and no management controls", () => {
      render(<MembersPage />, { wrapper });

      expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
      expect(screen.queryByTitle("Remove member")).not.toBeInTheDocument();
      // Role badges are still rendered.
      const ownerCard = cardFor("Owner User");
      expect(within(ownerCard).getByText("Owner")).toBeInTheDocument();
    });
  });

  describe("mutation error surfacing", () => {
    it("renders a visible inline error when removing a member fails", () => {
      mockUseRemoveMember.mockReturnValue({
        mutateAsync: jest.fn(),
        isPending: false,
        isError: true,
        error: new Error("Only owners can remove members"),
      });

      render(<MembersPage />, { wrapper });

      expect(screen.getAllByText("Only owners can remove members").length).toBeGreaterThan(0);
    });

    it("renders a visible inline error when a role change fails", () => {
      mockUseUpdateMemberRole.mockReturnValue({
        mutateAsync: jest.fn(),
        isPending: false,
        isError: true,
        error: new Error("Only owners can change owner roles"),
      });

      render(<MembersPage />, { wrapper });

      expect(screen.getAllByText("Only owners can change owner roles").length).toBeGreaterThan(0);
    });
  });
});
