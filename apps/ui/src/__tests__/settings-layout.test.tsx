import { render, screen } from "@testing-library/react";
import SettingsLayout from "@/app/(main)/settings/layout";

// Mock next/navigation
const mockPathname = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
}));

// Mock the Header component
jest.mock("@/components/layout/header", () => ({
  Header: ({ title }: { title: string }) => (
    <header data-testid="header">{title}</header>
  ),
}));

describe("SettingsLayout", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/settings/providers");
  });

  it("renders the Settings header", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>
    );

    expect(screen.getByTestId("header")).toHaveTextContent("Settings");
  });

  it("renders all navigation items", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>
    );

    expect(screen.getByText("LLM Providers")).toBeInTheDocument();
    expect(screen.getByText("API Keys")).toBeInTheDocument();
  });

  it("renders correct navigation links", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>
    );

    const providersLink = screen.getByRole("link", { name: /LLM Providers/i });
    const apiKeysLink = screen.getByRole("link", { name: /API Keys/i });

    expect(providersLink).toHaveAttribute("href", "/settings/providers");
    expect(apiKeysLink).toHaveAttribute("href", "/settings/api-keys");
  });

  it("highlights the active navigation item for providers", () => {
    mockPathname.mockReturnValue("/settings/providers");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>
    );

    const providersLink = screen.getByRole("link", { name: /LLM Providers/i });
    expect(providersLink).toHaveClass("bg-primary");
  });

  it("highlights the active navigation item for api-keys", () => {
    mockPathname.mockReturnValue("/settings/api-keys");
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>
    );

    const apiKeysLink = screen.getByRole("link", { name: /API Keys/i });
    expect(apiKeysLink).toHaveClass("bg-primary");
  });

  it("renders children content", () => {
    render(
      <SettingsLayout>
        <div data-testid="child-content">Test Child Content</div>
      </SettingsLayout>
    );

    expect(screen.getByTestId("child-content")).toBeInTheDocument();
    expect(screen.getByText("Test Child Content")).toBeInTheDocument();
  });

  it("does not render Members navigation (no API exists)", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>
    );

    expect(screen.queryByText("Members")).not.toBeInTheDocument();
  });

  it("has exactly 2 navigation items", () => {
    render(
      <SettingsLayout>
        <div>Test Content</div>
      </SettingsLayout>
    );

    const navLinks = screen.getAllByRole("link");
    expect(navLinks).toHaveLength(2);
  });
});
