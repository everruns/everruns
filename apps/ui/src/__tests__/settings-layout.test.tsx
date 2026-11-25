import { render, screen } from "@testing-library/react";
import SettingsLayout from "@/app/(main)/settings/layout";

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ hasRole: () => true }),
}));

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

const mockCanManageMcp = jest.fn(() => true);
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({
    can: (policy: string): boolean => policy === "mcp_server.manage" && mockCanManageMcp(),
  }),
}));

const mockPathname = jest.fn();
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlagsState: () => ({
    flags: {},
    isLoading: false,
  }),
}));

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn() }),
  usePathname: () => mockPathname(),
}));

describe("SettingsLayout", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/settings/providers");
    mockCanManageMcp.mockReturnValue(true);
  });

  it("lists MCP catalog under Organization for people who manage it", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const link = screen.getByRole("link", { name: "MCP catalog" });
    expect(link).toHaveAttribute("href", "/settings/mcp-catalog");
    expect(link.querySelector("svg")).toHaveAttribute("viewBox", "0 0 186 186");
  });

  it("hides MCP catalog from people who cannot manage it", () => {
    mockCanManageMcp.mockReturnValue(false);

    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.queryByRole("link", { name: "MCP catalog" })).not.toBeInTheDocument();
  });

  it("disables automatic prefetch for every Settings navigation link", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.getAllByRole("link")).toHaveLength(19);
    for (const link of screen.getAllByRole("link")) {
      expect(link).toHaveAttribute("data-prefetch", "false");
    }
  });

  it.each([
    ["/settings/personal-access-tokens", "Personal access tokens"],
    ["/settings/members", "Team members"],
    ["/settings/organization", "Organization"],
    ["/settings/chat", "Chat"],
    ["/settings/profile", "Account"],
    ["/settings/user-profile", "Profile"],
    ["/settings/connections", "Connections"],
    ["/settings/mcp-servers", "MCP servers"],
    ["/settings/ai-clients", "AI clients"],
  ])("highlights the active navigation item for %s", (pathname, linkName) => {
    mockPathname.mockReturnValue(pathname);
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.getByRole("link", { name: new RegExp(`^${linkName}$`, "i") })).toHaveClass(
      "border-primary",
    );
  });
});
