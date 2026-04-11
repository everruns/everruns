import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Sidebar } from "@/components/layout/sidebar";
import type { SidebarConfig, NavigationSection } from "@/components/layout/sidebar";
import { Settings, Zap } from "lucide-react";

// Mock next/navigation
const mockPathname = jest.fn();
const mockPush = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
  useRouter: () => ({
    push: mockPush,
    replace: jest.fn(),
    back: jest.fn(),
  }),
}));

// Mock next/image
jest.mock("next/image", () => ({
  __esModule: true,
  default: (props: { src: string; alt: string; width: number; height: number }) => (
    // eslint-disable-next-line nextjs/no-img-element
    <img src={props.src} alt={props.alt} width={props.width} height={props.height} />
  ),
}));

// Mock auth provider
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => ({
    user: null,
    requiresAuth: false,
    isAuthenticated: true,
    config: { mode: "none" },
    isLoading: false,
    logout: jest.fn(),
    logoutPending: false,
    createOrganization: undefined,
  }),
}));

// Mock org provider
const mockSetCurrentOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({
    currentOrg: { public_id: "org-123", name: "Test Org" },
    organizations: [{ public_id: "org-123", name: "Test Org" }],
    isLoading: false,
    setCurrentOrg: mockSetCurrentOrg,
  }),
}));

// Mock organizations hooks
const mockMutateAsync = jest.fn();
jest.mock("@/hooks/use-organizations", () => ({
  useCreateOrganization: () => ({
    mutateAsync: mockMutateAsync,
    isPending: false,
    isError: false,
    error: null,
  }),
}));

// Mock feature flags provider
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlags: () => ({
    global_chat: true,
    apps: true,
    mcp_endpoint: true,
  }),
}));

/// Mock LLM providers hook (default: providers configured, no warning)
const mockUseLlmProviders = jest.fn(() => ({
  data: [{ id: "provider-1", name: "Test Provider" }],
  isLoading: false,
  isError: false,
}));
jest.mock("@/hooks/use-llm-providers", () => ({
  useLlmProviders: () => mockUseLlmProviders(),
}));

describe("Sidebar", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/dashboard");
    mockPush.mockClear();
  });

  it("renders the Everruns logo and title", () => {
    render(<Sidebar />);

    expect(screen.getByText("Everruns")).toBeInTheDocument();
    expect(screen.getByAltText("Everruns")).toBeInTheDocument();
  });

  it("renders all navigation items", () => {
    render(<Sidebar />);

    expect(screen.getByText("Chat")).toBeInTheDocument();
    expect(screen.getByText("Dashboard")).toBeInTheDocument();
    expect(screen.getByText("Harnesses")).toBeInTheDocument();
    expect(screen.getByText("Agents")).toBeInTheDocument();
    expect(screen.getByText("Capabilities")).toBeInTheDocument();
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("renders correct navigation links", () => {
    render(<Sidebar />);

    const dashboardLink = screen.getByRole("link", { name: "Dashboard" });
    const harnessesLink = screen.getByRole("link", { name: "Harnesses" });
    const agentsLink = screen.getByRole("link", { name: "Agents" });
    const capabilitiesLink = screen.getByRole("link", { name: "Capabilities" });
    const settingsLink = screen.getByRole("link", { name: "Settings" });

    expect(dashboardLink).toHaveAttribute("href", "/dashboard");
    expect(harnessesLink).toHaveAttribute("href", "/harnesses");
    expect(agentsLink).toHaveAttribute("href", "/agents");
    expect(capabilitiesLink).toHaveAttribute("href", "/capabilities");
    expect(settingsLink).toHaveAttribute("href", "/settings");
  });

  it("does not render legacy navigation items", () => {
    render(<Sidebar />);

    expect(screen.queryByText("Runs")).not.toBeInTheDocument();
  });

  it("highlights the active navigation item", () => {
    mockPathname.mockReturnValue("/agents");
    render(<Sidebar />);

    const agentsLink = screen.getByRole("link", { name: /agents/i });
    expect(agentsLink).toHaveClass("border-l-primary");
    expect(agentsLink).toHaveClass("bg-card");
  });

  it("highlights navigation for nested routes", () => {
    mockPathname.mockReturnValue("/agents/123/sessions/456");
    render(<Sidebar />);

    const agentsLink = screen.getByRole("link", { name: /agents/i });
    expect(agentsLink).toHaveClass("border-l-primary");
    expect(agentsLink).toHaveClass("bg-card");
  });

  it("renders version in footer", () => {
    render(<Sidebar />);

    expect(screen.getByText(/^Everruns v\d+\.\d+\.\d+$/)).toBeInTheDocument();
  });

  it("has exactly 6 navigation items", () => {
    render(<Sidebar />);

    // Get nav links (excluding logo link)
    const navLinks = screen
      .getAllByRole("link")
      .filter(
        (link) =>
          link.getAttribute("href") !== "/dashboard" || link.textContent?.includes("Dashboard"),
      );
    // Filter to only nav items (Chat, Dashboard, Harnesses, Agents, Capabilities, Settings)
    const navItems = ["Chat", "Dashboard", "Harnesses", "Agents", "Capabilities", "Settings"];
    const foundNavLinks = navLinks.filter((link) =>
      navItems.some((item) => link.textContent?.includes(item)),
    );
    expect(foundNavLinks).toHaveLength(6);
  });

  it("renders section labels for Building Blocks and Durable Execution", () => {
    render(<Sidebar />);

    expect(screen.getByText("Building Blocks")).toBeInTheDocument();
    expect(screen.getByText("Durable Execution")).toBeInTheDocument();
  });

  it("hides Durable Execution navigation items by default (collapsed)", () => {
    render(<Sidebar />);

    expect(screen.queryByText("Overview")).not.toBeInTheDocument();
    expect(screen.queryByText("Workers")).not.toBeInTheDocument();
    expect(screen.queryByText("Workflows")).not.toBeInTheDocument();
    expect(screen.queryByText("Schedules")).not.toBeInTheDocument();
  });

  it("shows Durable Execution navigation items when expanded", () => {
    render(<Sidebar />);

    const toggle = screen.getByRole("button", { name: /durable execution/i });
    fireEvent.click(toggle);

    expect(screen.getByText("Overview")).toBeInTheDocument();
    expect(screen.getByText("Workers")).toBeInTheDocument();
    expect(screen.getByText("Workflows")).toBeInTheDocument();
    expect(screen.getByText("Schedules")).toBeInTheDocument();
  });

  it("collapses Durable Execution section again on second click", () => {
    render(<Sidebar />);

    const toggle = screen.getByRole("button", { name: /durable execution/i });
    fireEvent.click(toggle);
    expect(screen.getByText("Workers")).toBeInTheDocument();

    fireEvent.click(toggle);
    expect(screen.queryByText("Workers")).not.toBeInTheDocument();
  });

  it("nav element has overflow-y-auto and min-h-0 to prevent sidebar overflow", () => {
    render(<Sidebar />);

    const nav = screen.getByRole("navigation");
    expect(nav).toHaveClass("min-h-0");
    expect(nav).toHaveClass("overflow-y-auto");
  });

  it("highlights durable navigation for nested routes", () => {
    mockPathname.mockReturnValue("/durable/workers");
    render(<Sidebar />);

    // Expand the collapsed durable section first
    const toggle = screen.getByRole("button", { name: /durable execution/i });
    fireEvent.click(toggle);

    const workersLink = screen.getByRole("link", { name: /workers/i });
    expect(workersLink).toHaveClass("border-l-primary");
    expect(workersLink).toHaveClass("bg-card");
  });
});

describe("Sidebar with config", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/dashboard");
    mockPush.mockClear();
  });

  it("replaces navigation when config.navigation is provided", () => {
    const customNav: NavigationSection[] = [
      {
        label: "Custom",
        items: [{ name: "Custom Page", href: "/custom", icon: Settings }],
      },
    ];

    render(<Sidebar config={{ navigation: customNav }} />);

    expect(screen.getByText("Custom")).toBeInTheDocument();
    expect(screen.getByText("Custom Page")).toBeInTheDocument();
    // Default items should not appear
    expect(screen.queryByText("Building Blocks")).not.toBeInTheDocument();
    expect(screen.queryByText("Durable Execution")).not.toBeInTheDocument();
  });

  it("appends extraSections after default navigation", () => {
    const extra: NavigationSection[] = [
      {
        label: "Billing",
        items: [{ name: "Usage", href: "/billing/usage", icon: Zap }],
      },
    ];

    render(<Sidebar config={{ extraSections: extra }} />);

    // Default items still present
    expect(screen.getByText("Dashboard")).toBeInTheDocument();
    expect(screen.getByText("Building Blocks")).toBeInTheDocument();
    // Extra section appended
    expect(screen.getByText("Billing")).toBeInTheDocument();
    expect(screen.getByText("Usage")).toBeInTheDocument();
  });

  it("appends extraSections after custom navigation when both provided", () => {
    const customNav: NavigationSection[] = [
      { items: [{ name: "Home", href: "/home", icon: Settings }] },
    ];
    const extra: NavigationSection[] = [
      {
        label: "Extra",
        items: [{ name: "Extra Item", href: "/extra", icon: Zap }],
      },
    ];

    render(<Sidebar config={{ navigation: customNav, extraSections: extra }} />);

    expect(screen.getByText("Home")).toBeInTheDocument();
    expect(screen.getByText("Extra")).toBeInTheDocument();
    expect(screen.getByText("Extra Item")).toBeInTheDocument();
    // Default items replaced
    expect(screen.queryByText("Dashboard")).not.toBeInTheDocument();
  });

  it("does not render CreateOrganizationDialog when custom createOrg is provided", () => {
    const createOrgMock = jest.fn();
    const config: Partial<SidebarConfig> = {
      orgActions: { createOrg: createOrgMock },
    };

    render(<Sidebar config={config} />);

    // Dialog title should not be in the DOM when custom handler overrides default
    expect(screen.queryByText("Create a new organisation")).not.toBeInTheDocument();
    // The org selector trigger should still render
    expect(screen.getByText("Test Org")).toBeInTheDocument();
  });

  it("renders sections without labels (unlabeled sections)", () => {
    const customNav: NavigationSection[] = [
      {
        items: [{ name: "Unlabeled Item", href: "/unlabeled", icon: Settings }],
      },
    ];

    render(<Sidebar config={{ navigation: customNav }} />);

    expect(screen.getByText("Unlabeled Item")).toBeInTheDocument();
  });

  it("highlights active item with exact match when exact is true", () => {
    mockPathname.mockReturnValue("/custom");
    const customNav: NavigationSection[] = [
      {
        items: [
          { name: "Exact Match", href: "/custom", icon: Settings, exact: true },
          { name: "Child Page", href: "/custom/child", icon: Zap },
        ],
      },
    ];

    render(<Sidebar config={{ navigation: customNav }} />);

    const exactLink = screen.getByRole("link", { name: /exact match/i });
    expect(exactLink).toHaveClass("border-l-primary");
    expect(exactLink).toHaveClass("bg-card");
  });

  it("does not highlight exact-match item on child route", () => {
    mockPathname.mockReturnValue("/custom/child");
    const customNav: NavigationSection[] = [
      {
        items: [{ name: "Exact Only", href: "/custom", icon: Settings, exact: true }],
      },
    ];

    render(<Sidebar config={{ navigation: customNav }} />);

    const link = screen.getByRole("link", { name: /exact only/i });
    expect(link).not.toHaveClass("border-l-primary");
    expect(link).toHaveClass("border-l-transparent");
  });

  it("renders empty config same as no config", () => {
    const { container: withConfig } = render(<Sidebar config={{}} />);
    const { container: withoutConfig } = render(<Sidebar />);

    // Both should have the same nav items
    const withConfigLinks = withConfig.querySelectorAll("a");
    const withoutConfigLinks = withoutConfig.querySelectorAll("a");
    expect(withConfigLinks.length).toBe(withoutConfigLinks.length);
  });
});

describe("Create Organisation dialog", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/dashboard");
    mockPush.mockClear();
    mockMutateAsync.mockClear();
    mockSetCurrentOrg.mockClear();
  });

  it("redirects to setup page after successful org creation", async () => {
    mockMutateAsync.mockResolvedValue({
      id: "org_new123",
      name: "New Org",
    });

    render(<Sidebar />);

    // Open the org dropdown
    const orgTrigger = screen.getByText("Test Org");
    fireEvent.click(orgTrigger);

    // Click "Create Organisation"
    const createBtn = screen.getByText("Create Organisation");
    fireEvent.click(createBtn);

    // Fill in the org name
    const input = screen.getByPlaceholderText("Organisation name");
    fireEvent.change(input, { target: { value: "New Org" } });

    // Submit the form
    const submitBtn = screen.getByRole("button", { name: "Create" });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(mockMutateAsync).toHaveBeenCalledWith({ name: "New Org" });
      expect(mockSetCurrentOrg).toHaveBeenCalledWith({
        public_id: "org_new123",
        name: "New Org",
        role: "owner",
      });
      expect(mockPush).toHaveBeenCalledWith("/orgs/org_new123/setup");
    });
  });

  it("does not redirect when org creation is cancelled", () => {
    render(<Sidebar />);

    // Open the org dropdown
    const orgTrigger = screen.getByText("Test Org");
    fireEvent.click(orgTrigger);

    // Click "Create Organisation"
    const createBtn = screen.getByText("Create Organisation");
    fireEvent.click(createBtn);

    // Click Cancel
    const cancelBtn = screen.getByRole("button", { name: "Cancel" });
    fireEvent.click(cancelBtn);

    expect(mockPush).not.toHaveBeenCalled();
    expect(mockMutateAsync).not.toHaveBeenCalled();
  });

  it("shows chat warning badge when no LLM providers configured", () => {
    mockUseLlmProviders.mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
    });

    render(<Sidebar />);

    const warningBtn = screen.getByRole("button", {
      name: /no llm provider configured/i,
    });
    expect(warningBtn).toBeInTheDocument();

    // Restore default mock
    mockUseLlmProviders.mockReturnValue({
      data: [{ id: "provider-1", name: "Test Provider" }],
      isLoading: false,
      isError: false,
    });
  });

  it("hides chat warning badge when LLM providers are configured", () => {
    render(<Sidebar />);

    const warningBtn = screen.queryByRole("button", {
      name: /no llm provider configured/i,
    });
    expect(warningBtn).not.toBeInTheDocument();
  });
});
