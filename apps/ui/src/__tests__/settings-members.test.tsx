import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import MembersPage from "@/app/(main)/settings/members/page";

// Mock users data
const mockUsers = [
  {
    id: "user-1",
    email: "admin@example.com",
    name: "Admin User",
    avatar_url: "https://example.com/avatar1.jpg",
    roles: ["admin", "user"],
    auth_provider: "local",
    created_at: "2024-01-01T00:00:00Z",
  },
  {
    id: "user-2",
    email: "regular@example.com",
    name: "Regular User",
    avatar_url: null,
    roles: ["user"],
    auth_provider: "google",
    created_at: "2024-02-15T00:00:00Z",
  },
];

const mockUseUsers = jest.fn();
const mockUseAuth = jest.fn();

jest.mock("@/hooks/use-users", () => ({
  useUsers: (query?: { search?: string }) => mockUseUsers(query),
}));

jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => mockUseAuth(),
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

    // Default: auth required
    mockUseAuth.mockReturnValue({
      requiresAuth: true,
    });

    mockUseUsers.mockReturnValue({
      data: mockUsers,
      isLoading: false,
      error: null,
    });
  });

  it("renders Members section header", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Members")).toBeInTheDocument();
    expect(
      screen.getByText("View and manage team members.")
    ).toBeInTheDocument();
  });

  it("renders user cards with correct data", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Admin User")).toBeInTheDocument();
    expect(screen.getByText("Regular User")).toBeInTheDocument();
    expect(screen.getByText("admin@example.com")).toBeInTheDocument();
    expect(screen.getByText("regular@example.com")).toBeInTheDocument();
  });

  it("shows admin badge for admin users", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Admin")).toBeInTheDocument();
  });

  it("shows auth provider badges", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Local")).toBeInTheDocument();
    expect(screen.getByText("Google")).toBeInTheDocument();
  });

  it("renders search input", () => {
    render(<MembersPage />, { wrapper });

    expect(
      screen.getByPlaceholderText("Search by name or email...")
    ).toBeInTheDocument();
  });

  it("shows loading skeleton when loading", () => {
    mockUseUsers.mockReturnValue({
      data: [],
      isLoading: true,
      error: null,
    });

    render(<MembersPage />, { wrapper });

    const skeletons = document.querySelectorAll('[class*="animate-pulse"]');
    expect(skeletons.length).toBeGreaterThan(0);
  });

  it("shows empty state when no users exist", () => {
    mockUseUsers.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    });

    render(<MembersPage />, { wrapper });

    expect(screen.getByText("No members")).toBeInTheDocument();
  });

  it("shows error message when API fails", () => {
    mockUseUsers.mockReturnValue({
      data: [],
      isLoading: false,
      error: new Error("Network error"),
    });

    render(<MembersPage />, { wrapper });

    expect(screen.getByText(/Failed to load members/)).toBeInTheDocument();
  });

  it("shows authentication disabled message when auth is not required", () => {
    mockUseAuth.mockReturnValue({
      requiresAuth: false,
    });

    render(<MembersPage />, { wrapper });

    expect(screen.getByText("Authentication Disabled")).toBeInTheDocument();
    expect(
      screen.getByText(/Member management is only available when authentication is enabled/)
    ).toBeInTheDocument();
  });

  it("updates search when typing in search input", async () => {
    render(<MembersPage />, { wrapper });

    const searchInput = screen.getByPlaceholderText("Search by name or email...");
    fireEvent.change(searchInput, { target: { value: "admin" } });

    expect(searchInput).toHaveValue("admin");
  });

  it("shows member count", () => {
    render(<MembersPage />, { wrapper });

    expect(screen.getByText("2 members")).toBeInTheDocument();
  });

  it("shows joined date for users", () => {
    render(<MembersPage />, { wrapper });

    // Check for formatted dates (depends on locale, but should contain year)
    const joinedDates = screen.getAllByText(/Joined/);
    expect(joinedDates.length).toBe(2);
  });
});
