import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { Sidebar } from "@/components/layout/sidebar";
import type { SidebarConfig, NavigationSection } from "@/components/layout/sidebar";
import { DropdownMenuItem } from "@/components/ui/dropdown-menu";
import { Settings, Zap } from "lucide-react";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    prefetch,
    ...props
  }: React.AnchorHTMLAttributes<HTMLAnchorElement> & { prefetch?: boolean }) => (
    <a {...props} data-prefetch={prefetch === undefined ? undefined : String(prefetch)}>
      {children}
    </a>
  ),
}));

// Mock next/navigation
const mockPathname = jest.fn();
const mockPush = jest.fn();
const mockPrefetch = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
  useRouter: () => ({
    push: mockPush,
    prefetch: mockPrefetch,
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
const mockUseAuth = jest.fn();
jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => mockUseAuth(),
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

jest.mock("@/components/layout/mcp-connect-button", () => ({
  McpConnectButton: () => <button type="button" aria-label="Connect via MCP" />,
  McpConnectDialog: ({ open }: { open: boolean }) =>
    open ? (
      <div role="dialog" aria-label="MCP connection details">
        MCP connection details
      </div>
    ) : null,
  McpConnectMenuItem: ({ onSelect }: { onSelect: () => void }) => (
    <button role="menuitem" onClick={onSelect}>
      Connect via MCP
    </button>
  ),
}));

// Mock the live thread list: it queries sessions, which this suite does not stub.
const mockThreads = jest.fn<{ id: string; title: string }[], []>(() => []);
jest.mock("@/components/layout/sidebar-chat-threads", () => ({
  SidebarChatThreads: () => (
    <div data-testid="sidebar-chat-threads">
      {mockThreads().map((thread) => (
        <a key={thread.id} href={`/chats/${thread.id}`}>
          {thread.title}
        </a>
      ))}
    </div>
  ),
}));

jest.mock("@/components/layout/notification-bell", () => ({
  NotificationBell: () => <button type="button" aria-label="Notifications" />,
  NotificationIndicator: () => <span aria-label="Unread notifications" />,
  NotificationMenuSub: () => <div>Notifications</div>,
}));

// Mock feature flags provider
const mockFeatureFlags = {
  notifications: true,
  evals: true,
  skills: true,
  memory: true,
  knowledge: true,
  plugins: true,
  app_budgets: true,
  agent_versions: true,
  voice: true,
  agent_delegation: true,
  observers: true,
  public_chat: true,
  webmcp: true,
};
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlags: () => mockFeatureFlags,
  useFeatureFlag: (flag: string) => mockFeatureFlags[flag as keyof typeof mockFeatureFlags],
}));

/// Mock LLM providers hook (default: providers configured, no warning)
const mockUseProviders = jest.fn(() => ({
  data: [{ id: "provider-1", name: "Test Provider" }],
  isLoading: false,
  isError: false,
}));
jest.mock("@/hooks/use-providers", () => ({
  useProviders: () => mockUseProviders(),
}));

const mockUsePolicies = jest.fn(() => ({
  data: { policies: { "durable.view": true } },
  // Annotate the return as boolean so it matches the real usePolicies().can
  // signature; without it, TS 5.5+ infers a `policyId is "durable.view"`
  // predicate from the comparison, which then rejects plain boolean mocks.
  can: (policyId: string): boolean => policyId === "durable.view",
}));
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => mockUsePolicies(),
}));

describe("Sidebar", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/dashboard");
    Object.assign(mockFeatureFlags, {
      notifications: true,
      evals: true,
      skills: true,
      memory: true,
      knowledge: true,
      plugins: true,
      app_budgets: true,
      agent_versions: true,
      voice: true,
      agent_delegation: true,
      observers: true,
      public_chat: true,
      webmcp: true,
    });
    mockPush.mockClear();
    mockPrefetch.mockClear();
    mockThreads.mockReturnValue([]);
    mockUsePolicies.mockReturnValue({
      data: { policies: { "durable.view": true } },
      can: (policyId: string) => policyId === "durable.view",
    });
    mockUseAuth.mockReturnValue({
      user: null,
      requiresAuth: false,
      isAuthenticated: true,
      config: { mode: "none" },
      isLoading: false,
      logout: jest.fn(),
      logoutPending: false,
      createOrganization: undefined,
    });
  });

  it("renders the Everruns logo and title", () => {
    render(<Sidebar />);

    expect(screen.getByText("Everruns")).toBeInTheDocument();
    expect(screen.getByAltText("Everruns")).toBeInTheDocument();
  });

  it("keeps search row focused on search only", () => {
    render(<Sidebar />);

    const brandRow = screen.getByAltText("Everruns").closest("a")?.parentElement;
    const searchRow = screen.getByRole("search", { name: "Sidebar search" });

    expect(brandRow).not.toBeNull();
    expect(within(searchRow).getByRole("button", { name: /search/i })).toBeInTheDocument();
    expect(within(brandRow!).queryByLabelText("Connect via MCP")).not.toBeInTheDocument();
    expect(within(brandRow!).queryByLabelText("Notifications")).not.toBeInTheDocument();
    expect(within(searchRow).queryByLabelText("Connect via MCP")).not.toBeInTheDocument();
    expect(within(searchRow).queryByLabelText("Notifications")).not.toBeInTheDocument();
  });

  it("renders notifications as an account-row indicator and MCP in the profile menu", () => {
    mockUseAuth.mockReturnValue({
      user: { name: "Test User", email: "test@example.com", avatar_url: null },
      requiresAuth: true,
      isAuthenticated: true,
      config: { mode: "password" },
      isLoading: false,
      logout: jest.fn(),
      logoutPending: false,
      createOrganization: undefined,
    });

    render(<Sidebar />);

    const footer = screen.getByRole("contentinfo", { name: "Sidebar footer" });
    expect(within(footer).getByRole("button", { name: /test user/i })).toBeInTheDocument();
    expect(within(footer).getByLabelText("Unread notifications")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /test user/i }));
    expect(screen.getByText("Notifications")).toBeInTheDocument();
    expect(screen.getByText("Connect via MCP")).toBeInTheDocument();
  });

  it("renders a local identity menu with applicable actions in no-auth mode", async () => {
    mockUseAuth.mockReturnValue({
      user: { name: "Anonymous", email: "anonymous@local", avatar_url: null },
      requiresAuth: false,
      isAuthenticated: true,
      config: { mode: "none" },
      isLoading: false,
      logout: jest.fn(),
      logoutPending: false,
      createOrganization: undefined,
    });

    render(<Sidebar />);

    const footer = screen.getByRole("contentinfo", { name: "Sidebar footer" });
    const trigger = within(footer).getByRole("button", { name: /anonymous/i });
    expect(trigger).toHaveTextContent("Local user");

    fireEvent.click(trigger);
    expect(screen.getByRole("menuitem", { name: "Profile" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Connect via MCP" })).toBeInTheDocument();
    expect(
      screen.queryByRole("menuitem", { name: /personal access tokens/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: /sign out/i })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("menuitem", { name: "Connect via MCP" }));
    expect(
      await screen.findByRole("dialog", { name: /mcp connection details/i }),
    ).toBeInTheDocument();
  });

  it("keeps MCP Connect visible without endpoint feature flag data", () => {
    mockUseAuth.mockReturnValue({
      user: { name: "Anonymous", email: "anonymous@local", avatar_url: null },
      requiresAuth: false,
      isAuthenticated: true,
      config: { mode: "none" },
      isLoading: false,
      logout: jest.fn(),
      logoutPending: false,
      createOrganization: undefined,
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: /anonymous/i }));

    expect(screen.getByRole("menuitem", { name: "Connect via MCP" })).toBeInTheDocument();
  });

  it("preserves authentication-only actions in authenticated mode", () => {
    mockUseAuth.mockReturnValue({
      user: { name: "Test User", email: "test@example.com", avatar_url: null },
      requiresAuth: true,
      isAuthenticated: true,
      config: { mode: "password" },
      isLoading: false,
      logout: jest.fn(),
      logoutPending: false,
      createOrganization: undefined,
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: /test user/i }));

    expect(screen.getByRole("menuitem", { name: /personal access tokens/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /sign out/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Connect via MCP" })).toBeInTheDocument();
  });

  it("renders only the version when no current user is available", () => {
    render(<Sidebar />);

    const footer = screen.getByRole("contentinfo", { name: "Sidebar footer" });
    expect(within(footer).getByText(/^Everruns v\d+\.\d+\.\d+$/)).toBeInTheDocument();
    expect(within(footer).queryByRole("button")).not.toBeInTheDocument();
  });

  it("routes the Profile menu item to /settings/profile", () => {
    mockUseAuth.mockReturnValue({
      user: { name: "Test User", email: "test@example.com", avatar_url: null },
      requiresAuth: true,
      isAuthenticated: true,
      config: { mode: "password" },
      isLoading: false,
      logout: jest.fn(),
      logoutPending: false,
      createOrganization: undefined,
    });

    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: /test user/i }));
    fireEvent.click(screen.getByRole("menuitem", { name: /profile/i }));

    expect(mockPush).toHaveBeenCalledWith("/settings/profile");
  });

  it("renders all navigation items", () => {
    render(<Sidebar />);

    expect(screen.getByText("Chats")).toBeInTheDocument();
    expect(screen.getByText("Sessions")).toBeInTheDocument();
    expect(screen.getByText("Reports")).toBeInTheDocument();
    expect(screen.getByText("Harnesses")).toBeInTheDocument();
    expect(screen.getByText("Agents")).toBeInTheDocument();
    expect(screen.getByText("Identities")).toBeInTheDocument();
    expect(screen.getByText("Knowledge indexes")).toBeInTheDocument();
    expect(screen.getByText("Memory")).toBeInTheDocument();
    expect(screen.getByText("Apps")).toBeInTheDocument();
    expect(screen.getByText("Models")).toBeInTheDocument();
    expect(screen.getByText("MCP servers")).toBeInTheDocument();
    expect(screen.getByText("Skills")).toBeInTheDocument();
    expect(screen.getByText("Capabilities")).toBeInTheDocument();
    expect(screen.getByText("Plugins")).toBeInTheDocument();
    expect(screen.getByText("Evals")).toBeInTheDocument();
    expect(screen.getByText("Observers")).toBeInTheDocument();
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("hides opt-in feature navigation when its flags are disabled", () => {
    Object.assign(mockFeatureFlags, {
      evals: false,
      skills: false,
      memory: false,
      knowledge: false,
      plugins: false,
      observers: false,
    });

    render(<Sidebar />);

    expect(screen.queryByText("Evals")).not.toBeInTheDocument();
    expect(screen.queryByText("Skills")).not.toBeInTheDocument();
    expect(screen.queryByText("Memory")).not.toBeInTheDocument();
    expect(screen.queryByText("Knowledge indexes")).not.toBeInTheDocument();
    expect(screen.queryByText("Plugins")).not.toBeInTheDocument();
    expect(screen.queryByText("Quality")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Chats" })).toHaveAttribute("href", "/chats");
  });

  it("renders correct navigation links", () => {
    render(<Sidebar />);

    const chatsLink = screen.getByRole("link", { name: /chats/i });
    const harnessesLink = screen.getByRole("link", { name: "Harnesses" });
    const agentsLink = screen.getByRole("link", { name: "Agents" });
    const memoryLink = screen.getByRole("link", { name: "Memory" });
    const modelsLink = screen.getByRole("link", { name: "Models" });
    const capabilitiesLink = screen.getByRole("link", { name: "Capabilities" });
    const identitiesLink = screen.getByRole("link", { name: "Identities" });
    const knowledgeLink = screen.getByRole("link", { name: "Knowledge indexes" });
    const mcpServersLink = screen.getByRole("link", { name: "MCP servers" });
    const settingsLink = screen.getByRole("link", { name: "Settings" });

    expect(chatsLink).toHaveAttribute("href", "/chats");
    expect(identitiesLink).toHaveAttribute("href", "/agent-identities");
    expect(knowledgeLink).toHaveAttribute("href", "/knowledge-indexes");
    expect(mcpServersLink).toHaveAttribute("href", "/mcp-servers");
    expect(harnessesLink).toHaveAttribute("href", "/harnesses");
    expect(agentsLink).toHaveAttribute("href", "/agents");
    expect(memoryLink).toHaveAttribute("href", "/memory");
    expect(modelsLink).toHaveAttribute("href", "/models");
    expect(capabilitiesLink).toHaveAttribute("href", "/capabilities");
    expect(settingsLink).toHaveAttribute("href", "/settings/organization");
  });

  it("renders distinct semantic icons for registry navigation", () => {
    render(<Sidebar />);

    const icon = (name: string) => screen.getByRole("link", { name }).querySelector("svg");

    expect(icon("Skills")).toHaveClass("lucide-book-open");
    expect(icon("Capabilities")).toHaveClass("lucide-blocks");
    expect(icon("Plugins")).toHaveClass("lucide-plug");
    expect(icon("MCP servers")).toHaveAttribute("viewBox", "0 0 186 186");
  });

  it("disables automatic viewport prefetch for every sidebar navigation link", () => {
    render(<Sidebar />);

    expect(screen.getByAltText("Everruns").closest("a")).toHaveAttribute("data-prefetch", "false");
    const navigation = screen.getByRole("navigation");
    const links = within(navigation).getAllByRole("link");
    for (const link of links) {
      expect(link).toHaveAttribute("data-prefetch", "false");
    }
  });

  it("prefetches only the intended link on hover or keyboard focus", () => {
    render(<Sidebar />);

    fireEvent.mouseEnter(screen.getByRole("link", { name: "Sessions" }));
    expect(mockPrefetch).toHaveBeenCalledTimes(1);
    expect(mockPrefetch).toHaveBeenLastCalledWith("/sessions");

    fireEvent.focus(screen.getByRole("link", { name: "Agents" }));
    expect(mockPrefetch).toHaveBeenCalledTimes(2);
    expect(mockPrefetch).toHaveBeenLastCalledWith("/agents");
  });

  it("preserves Settings no-prefetch behavior on hover and focus", () => {
    render(<Sidebar />);

    const settingsLink = screen.getByRole("link", { name: "Settings" });
    fireEvent.mouseEnter(settingsLink);
    fireEvent.focus(settingsLink);

    expect(mockPrefetch).not.toHaveBeenCalled();
  });

  it("does not render legacy navigation items", () => {
    render(<Sidebar />);

    expect(screen.queryByText("Runs")).not.toBeInTheDocument();
    expect(screen.queryByText("Building Blocks")).not.toBeInTheDocument();
    expect(screen.queryByText("Dashboard")).not.toBeInTheDocument();
    expect(screen.queryByText("Work")).not.toBeInTheDocument();
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

  it.each([
    "/settings/organization",
    "/settings/providers",
    "/settings/members",
    "/settings/features",
    "/settings/payments",
    "/settings/profile",
    "/settings/connections",
    "/settings/personal-access-tokens",
  ])("keeps Settings active on %s", (pathname) => {
    mockPathname.mockReturnValue(pathname);
    render(<Sidebar />);

    expect(screen.getByRole("link", { name: "Settings" })).toHaveClass("border-l-primary");
  });

  it("renders version in footer", () => {
    render(<Sidebar />);

    expect(screen.getByText(/^Everruns v\d+\.\d+\.\d+$/)).toBeInTheDocument();
  });

  // Pure-styling and hardcoded-count change-detectors removed (compact-shell
  // utility classes, exact nav-item count, nav overflow classes): they broke on
  // cosmetic refactors while asserting no behavior. Active-state/routing tests
  // that verify *which* item is highlighted are kept below.

  it("renders the grouped section labels and Durable Execution", () => {
    render(<Sidebar />);

    expect(screen.getByText("Operational")).toBeInTheDocument();
    expect(screen.getByText("Building")).toBeInTheDocument();
    expect(screen.getByText("Registries")).toBeInTheDocument();
    expect(screen.getByText("Quality")).toBeInTheDocument();
    expect(screen.getByText("Durable Execution")).toBeInTheDocument();
  });

  it("leads with Chats above the first labelled group", () => {
    render(<Sidebar />);

    const navigation = screen.getByRole("navigation");
    const links = within(navigation).getAllByRole("link");
    expect(links[0]).toHaveAttribute("href", "/chats");
  });

  it("hangs the thread list directly under the Chats entry", () => {
    mockThreads.mockReturnValue([{ id: "sess_1", title: "Standup" }]);

    render(<Sidebar />);

    const navigation = screen.getByRole("navigation");
    const links = within(navigation).getAllByRole("link");
    expect(links[0]).toHaveAttribute("href", "/chats");
    expect(links[1]).toHaveAttribute("href", "/chats/sess_1");
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

  it("hides Durable Execution when durable policy is denied", () => {
    mockUsePolicies.mockReturnValue({
      data: { policies: { "durable.view": false } },
      can: () => false,
    });

    render(<Sidebar />);

    expect(screen.queryByText("Durable Execution")).not.toBeInTheDocument();
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
    expect(screen.queryByText("Building")).not.toBeInTheDocument();
    expect(screen.queryByText("Durable Execution")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Custom Page" })).toHaveAttribute(
      "data-prefetch",
      "false",
    );
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
    expect(screen.getByText("Sessions")).toBeInTheDocument();
    expect(screen.getByText("Building")).toBeInTheDocument();
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
    expect(screen.queryByText("Sessions")).not.toBeInTheDocument();
  });

  it("does not render CreateOrganizationDialog when custom createOrg is provided", () => {
    const createOrgMock = jest.fn();
    const config: Partial<SidebarConfig> = {
      orgActions: { createOrg: createOrgMock },
    };

    render(<Sidebar config={config} />);

    // Dialog title should not be in the DOM when custom handler overrides default
    expect(screen.queryByText("Create a new organization")).not.toBeInTheDocument();
    // The org selector trigger should still render
    expect(screen.getByText("Test Org")).toBeInTheDocument();
  });

  it("appends custom profile menu items when config.profileMenu.items is provided", () => {
    mockUseAuth.mockReturnValue({
      user: { name: "Test User", email: "test@example.com", avatar_url: null },
      requiresAuth: true,
      isAuthenticated: true,
      config: { mode: "password" },
      isLoading: false,
      logout: jest.fn(),
      logoutPending: false,
      createOrganization: undefined,
    });

    const config: Partial<SidebarConfig> = {
      profileMenu: {
        items: ({ navigate }) => (
          <DropdownMenuItem onClick={() => navigate("/billing")}>Billing</DropdownMenuItem>
        ),
      },
    };

    render(<Sidebar config={config} />);

    fireEvent.click(screen.getByRole("button", { name: /test user/i }));

    const billingItem = screen.getByText("Billing");
    expect(billingItem).toBeInTheDocument();

    fireEvent.click(billingItem);
    expect(mockPush).toHaveBeenCalledWith("/billing");
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

describe("Create Organization dialog", () => {
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

    // Click "Create Organization"
    const createBtn = screen.getByText("Create Organization");
    fireEvent.click(createBtn);

    // Fill in the org name
    const input = screen.getByPlaceholderText("Organization name");
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

    // Click "Create Organization"
    const createBtn = screen.getByText("Create Organization");
    fireEvent.click(createBtn);

    // Click Cancel
    const cancelBtn = screen.getByRole("button", { name: "Cancel" });
    fireEvent.click(cancelBtn);

    expect(mockPush).not.toHaveBeenCalled();
    expect(mockMutateAsync).not.toHaveBeenCalled();
  });

  it("shows chat warning badge when no LLM providers configured", () => {
    mockUseProviders.mockReturnValue({
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
    mockUseProviders.mockReturnValue({
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
