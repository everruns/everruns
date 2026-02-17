import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import MembersPage from "@/app/(main)/settings/members/page";

// Mock members data
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

    mockUseAuth.mockReturnValue({
      user: { id: "user-1" },
      requiresAuth: true,
    });

    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org-123", name: "Test Org", role: "owner" },
      hasRole: (role: string) => {
        const levels: Record<string, number> = { owner: 3, admin: 2, member: 1 };
        return (levels["owner"] ?? 0) >= (levels[role] ?? 0);
      },
      isLoading: false,
    });

    mockUseMembers.mockReturnValue({
      data: mockMembers,
      isLoading: false,
      error: null,
    });

    mockUseUpdateMemberRole.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });

    mockUseRemoveMember.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
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

    // Current user (user-1) should have Owner badge (not a dropdown)
    expect(screen.getByText("Owner")).toBeInTheDocument();
    // And "You" badge
    expect(screen.getByText("You")).toBeInTheDocument();
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

    expect(screen.getByText("2 members")).toBeInTheDocument();
  });

  it("shows joined date for members", () => {
    render(<MembersPage />, { wrapper });

    const joinedDates = screen.getAllByText(/Joined/);
    expect(joinedDates.length).toBe(2);
  });

  it("shows remove button for non-self members when user can manage", () => {
    render(<MembersPage />, { wrapper });

    // Remove button should exist for the non-self member
    expect(screen.getByTitle("Remove member")).toBeInTheDocument();
  });

  it("hides role editing when user lacks admin role", () => {
    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org-123", name: "Test Org", role: "member" },
      hasRole: () => false,
      isLoading: false,
    });

    mockUseAuth.mockReturnValue({
      user: { id: "user-2" },
      requiresAuth: true,
    });

    render(<MembersPage />, { wrapper });

    // Should show badges for all members, not dropdowns
    expect(screen.getByText("Owner")).toBeInTheDocument();
    expect(screen.getByText("Member")).toBeInTheDocument();
    // No remove button
    expect(screen.queryByTitle("Remove member")).not.toBeInTheDocument();
  });

  it("shows org name in subtitle", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Test Org")).toBeInTheDocument();
  });
});
