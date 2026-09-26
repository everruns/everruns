import { render, screen } from "@testing-library/react";
import SettingsLayout from "@/app/(main)/settings/layout";

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
    mockNotFound.mockClear();
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

  it("renders the Settings header", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Settings");
  });

  it("renders all navigation items", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.queryByText("General")).not.toBeInTheDocument();
    expect(screen.getAllByText("Organization")).toHaveLength(2);
    expect(screen.getByText("LLM Providers")).toBeInTheDocument();
    expect(screen.getByText("Members")).toBeInTheDocument();
    expect(screen.getByText("Payments")).toBeInTheDocument();
    expect(screen.getByText("Profile")).toBeInTheDocument();
    expect(screen.getByText("Connections")).toBeInTheDocument();
    expect(screen.getByText("Personal access tokens")).toBeInTheDocument();
  });

  it("renders section labels for Organization and Personal", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(
      screen.getAllByText("Organization").some((node) => node.classList.contains("uppercase")),
    ).toBe(true);
    expect(screen.getByText("Personal")).toBeInTheDocument();
  });

  it("renders correct navigation links", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const organizationLink = screen.getByRole("link", { name: /Organization/i });
    const providersLink = screen.getByRole("link", { name: /LLM Providers/i });
    const membersLink = screen.getByRole("link", { name: /Members/i });
    const paymentsLink = screen.getByRole("link", { name: /Payments/i });
    const profileLink = screen.getByRole("link", { name: /Profile/i });
    const connectionsLink = screen.getByRole("link", { name: /Connections/i });
    const apiKeysLink = screen.getByRole("link", { name: /Personal access tokens/i });

    expect(organizationLink).toHaveAttribute("href", "/settings/organization");
    expect(providersLink).toHaveAttribute("href", "/settings/providers");
    expect(membersLink).toHaveAttribute("href", "/settings/members");
    expect(paymentsLink).toHaveAttribute("href", "/settings/payments");
    expect(profileLink).toHaveAttribute("href", "/settings/profile");
    expect(connectionsLink).toHaveAttribute("href", "/settings/connections");
    expect(apiKeysLink).toHaveAttribute("href", "/settings/personal-access-tokens");
  });

  it("disables automatic prefetch for every Settings navigation link", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(screen.getAllByRole("link")).toHaveLength(8);
    for (const link of screen.getAllByRole("link")) {
      expect(link).toHaveAttribute("data-prefetch", "false");
    }
  });

  it.each([
    ["/settings/providers", "LLM Providers"],
    ["/settings/personal-access-tokens", "Personal access tokens"],
    ["/settings/members", "Members"],
    ["/settings/organization", "Organization"],
    ["/settings/profile", "Profile"],
    ["/settings/connections", "Connections"],
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

  it("renders children content", () => {
    render(
      <SettingsLayout>
        <div data-testid="child-content">Test Child Content</div>
      </SettingsLayout>,
    );

    expect(screen.getByTestId("child-content")).toBeInTheDocument();
    expect(screen.getByText("Test Child Content")).toBeInTheDocument();
  });

  it("groups items under correct sections", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const orgLabel = screen
      .getAllByText("Organization")
      .find((node) => node.classList.contains("uppercase"))!;
    const personalLabel = screen.getByText("Personal");

    // Organization section contains its items
    const orgSection = orgLabel.closest("div[class]")!.parentElement!;
    expect(orgSection).not.toHaveTextContent("General");
    expect(orgSection).toHaveTextContent("Organization");
    expect(orgSection).toHaveTextContent("LLM Providers");
    expect(orgSection).toHaveTextContent("Members");
    expect(orgSection).toHaveTextContent("Features");
    expect(orgSection).toHaveTextContent("Payments");
    expect(orgSection).not.toHaveTextContent("Connections");
    expect(orgSection).not.toHaveTextContent("Personal access tokens");

    // Personal section contains its items
    const personalSection = personalLabel.closest("div[class]")!.parentElement!;
    expect(personalSection).toHaveTextContent("Profile");
    expect(personalSection).toHaveTextContent("Connections");
    expect(personalSection).toHaveTextContent("Personal access tokens");
    expect(personalSection).not.toHaveTextContent("Organization");
    expect(personalSection).not.toHaveTextContent("Members");
  });

  it("applies inactive styles to non-active items", () => {
    mockPathname.mockReturnValue("/settings/providers");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const organizationLink = screen.getByRole("link", { name: /Organization/i });
    const membersLink = screen.getByRole("link", { name: /Members/i });
    const apiKeysLink = screen.getByRole("link", { name: /Personal access tokens/i });

    expect(organizationLink).toHaveClass("border-transparent");
    expect(membersLink).toHaveClass("border-transparent");
    expect(apiKeysLink).toHaveClass("border-transparent");
  });

});
