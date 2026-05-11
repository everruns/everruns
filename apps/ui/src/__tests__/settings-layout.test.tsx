import { render, screen } from "@testing-library/react";
import SettingsLayout from "@/app/(main)/settings/layout";

// Mock next/navigation
const mockPathname = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
}));

describe("SettingsLayout", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/settings/providers");
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
    expect(screen.getAllByText("Organisation")).toHaveLength(2);
    expect(screen.getByText("LLM Providers")).toBeInTheDocument();
    expect(screen.getByText("Members")).toBeInTheDocument();
    expect(screen.getByText("Payments")).toBeInTheDocument();
    expect(screen.getByText("Profile")).toBeInTheDocument();
    expect(screen.getByText("Connections")).toBeInTheDocument();
    expect(screen.getByText("API Keys")).toBeInTheDocument();
  });

  it("renders section labels for Organisation and Personal", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    expect(
      screen.getAllByText("Organisation").some((node) => node.classList.contains("uppercase")),
    ).toBe(true);
    expect(screen.getByText("Personal")).toBeInTheDocument();
  });

  it("renders correct navigation links", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const organisationLink = screen.getByRole("link", { name: /Organisation/i });
    const providersLink = screen.getByRole("link", { name: /LLM Providers/i });
    const membersLink = screen.getByRole("link", { name: /Members/i });
    const paymentsLink = screen.getByRole("link", { name: /Payments/i });
    const profileLink = screen.getByRole("link", { name: /Profile/i });
    const connectionsLink = screen.getByRole("link", { name: /Connections/i });
    const apiKeysLink = screen.getByRole("link", { name: /API Keys/i });

    expect(organisationLink).toHaveAttribute("href", "/settings/organization");
    expect(providersLink).toHaveAttribute("href", "/settings/providers");
    expect(membersLink).toHaveAttribute("href", "/settings/members");
    expect(paymentsLink).toHaveAttribute("href", "/settings/payments");
    expect(profileLink).toHaveAttribute("href", "/settings/profile");
    expect(connectionsLink).toHaveAttribute("href", "/settings/connections");
    expect(apiKeysLink).toHaveAttribute("href", "/settings/api-keys");
  });

  it("highlights the active navigation item for providers", () => {
    mockPathname.mockReturnValue("/settings/providers");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const providersLink = screen.getByRole("link", { name: /LLM Providers/i });
    expect(providersLink).toHaveClass("border-accent");
  });

  it("highlights the active navigation item for api-keys", () => {
    mockPathname.mockReturnValue("/settings/api-keys");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const apiKeysLink = screen.getByRole("link", { name: /API Keys/i });
    expect(apiKeysLink).toHaveClass("border-accent");
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

  it("highlights the active navigation item for members", () => {
    mockPathname.mockReturnValue("/settings/members");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const membersLink = screen.getByRole("link", { name: /Members/i });
    expect(membersLink).toHaveClass("border-accent");
  });

  it("has exactly 7 navigation items", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const navLinks = screen.getAllByRole("link");
    expect(navLinks).toHaveLength(7);
  });

  it("groups items under correct sections", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const orgLabel = screen
      .getAllByText("Organisation")
      .find((node) => node.classList.contains("uppercase"))!;
    const personalLabel = screen.getByText("Personal");

    // Organisation section contains its items
    const orgSection = orgLabel.closest("div[class]")!.parentElement!;
    expect(orgSection).not.toHaveTextContent("General");
    expect(orgSection).toHaveTextContent("Organisation");
    expect(orgSection).toHaveTextContent("LLM Providers");
    expect(orgSection).toHaveTextContent("Members");
    expect(orgSection).toHaveTextContent("Payments");
    expect(orgSection).not.toHaveTextContent("Connections");
    expect(orgSection).not.toHaveTextContent("API Keys");

    // Personal section contains its items
    const personalSection = personalLabel.closest("div[class]")!.parentElement!;
    expect(personalSection).toHaveTextContent("Profile");
    expect(personalSection).toHaveTextContent("Connections");
    expect(personalSection).toHaveTextContent("API Keys");
    expect(personalSection).not.toHaveTextContent("Organisation");
    expect(personalSection).not.toHaveTextContent("Members");
  });

  it("renders section labels with uppercase styling", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const orgLabel = screen
      .getAllByText("Organisation")
      .find((node) => node.classList.contains("uppercase"))!;
    const personalLabel = screen.getByText("Personal");

    expect(orgLabel).toHaveClass("uppercase");
    expect(orgLabel).toHaveClass("tracking-wider");
    expect(orgLabel).toHaveClass("text-xs");
    expect(orgLabel).toHaveClass("font-semibold");

    expect(personalLabel).toHaveClass("uppercase");
    expect(personalLabel).toHaveClass("tracking-wider");
  });

  it("applies inactive styles to non-active items", () => {
    mockPathname.mockReturnValue("/settings/providers");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const organisationLink = screen.getByRole("link", { name: /Organisation/i });
    const membersLink = screen.getByRole("link", { name: /Members/i });
    const apiKeysLink = screen.getByRole("link", { name: /API Keys/i });

    expect(organisationLink).toHaveClass("border-transparent");
    expect(membersLink).toHaveClass("border-transparent");
    expect(apiKeysLink).toHaveClass("border-transparent");
  });

  it("highlights active item for Organisation page", () => {
    mockPathname.mockReturnValue("/settings/organization");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const organisationLink = screen.getByRole("link", { name: /Organisation/i });
    expect(organisationLink).toHaveClass("border-accent");
  });

  it("highlights active item for Profile page", () => {
    mockPathname.mockReturnValue("/settings/profile");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const profileLink = screen.getByRole("link", { name: /Profile/i });
    expect(profileLink).toHaveClass("border-accent");
  });

  it("highlights active item for Connections page", () => {
    mockPathname.mockReturnValue("/settings/connections");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>,
    );

    const connectionsLink = screen.getByRole("link", { name: /Connections/i });
    expect(connectionsLink).toHaveClass("border-accent");
  });
});
