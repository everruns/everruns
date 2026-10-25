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
const mockMachinePaymentsEnabled = jest.fn(() => true);
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlagsState: () => ({
    flags: { machine_payments: mockMachinePaymentsEnabled() },
    isLoading: false,
  }),
}));

const mockNotFound = jest.fn(() => {
  throw new Error("NEXT_NOT_FOUND");
});
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
  notFound: () => mockNotFound(),
}));

describe("SettingsLayout", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/settings/providers");
    mockMachinePaymentsEnabled.mockReturnValue(true);
    mockCanManageMcp.mockReturnValue(true);
    mockNotFound.mockClear();
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

  it("hides Payments navigation when machine payments are disabled", () => {
    mockMachinePaymentsEnabled.mockReturnValue(false);

    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.queryByRole("link", { name: /Payments/i })).not.toBeInTheDocument();
  });

  it("returns not found for the Payments page when machine payments are disabled", () => {
    mockPathname.mockReturnValue("/settings/payments");
    mockMachinePaymentsEnabled.mockReturnValue(false);

    expect(() =>
      render(
        <SettingsLayout>
          <div>Payment custody form</div>
        </SettingsLayout>,
      ),
    ).toThrow("NEXT_NOT_FOUND");
    expect(mockNotFound).toHaveBeenCalled();
    expect(screen.queryByText("Payment custody form")).not.toBeInTheDocument();
  });

  it("disables automatic prefetch for every Settings navigation link", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.getAllByRole("link")).toHaveLength(10);
    for (const link of screen.getAllByRole("link")) {
      expect(link).toHaveAttribute("data-prefetch", "false");
    }
  });

  it.each([
    ["/settings/personal-access-tokens", "Personal access tokens"],
    ["/settings/members", "Team members"],
    ["/settings/organization", "Organization"],
    ["/settings/profile", "Account"],
    ["/settings/agent-experience", "My agent experience"],
  ])("highlights the active navigation item for %s", (pathname, linkName) => {
    mockPathname.mockReturnValue(pathname);
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.getByRole("link", { name: new RegExp(linkName, "i") })).toHaveClass(
      "border-accent",
    );
  });
});
