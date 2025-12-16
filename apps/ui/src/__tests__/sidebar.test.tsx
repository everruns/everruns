import { render, screen } from "@testing-library/react";
import { Sidebar } from "@/components/layout/sidebar";

// Mock next/navigation
const mockPathname = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
}));

// Mock next/image
jest.mock("next/image", () => ({
  __esModule: true,
  default: (props: { src: string; alt: string; width: number; height: number }) => (
    // eslint-disable-next-line @next/next/no-img-element
    <img src={props.src} alt={props.alt} width={props.width} height={props.height} />
  ),
}));

describe("Sidebar", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/dashboard");
  });

  it("renders the Everruns logo and title", () => {
    render(<Sidebar />);

    expect(screen.getByText("Everruns")).toBeInTheDocument();
    expect(screen.getByAltText("Everruns")).toBeInTheDocument();
  });

  it("renders all navigation items", () => {
    render(<Sidebar />);

    expect(screen.getByText("Dashboard")).toBeInTheDocument();
    expect(screen.getByText("Harnesses")).toBeInTheDocument();
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("renders correct navigation links", () => {
    render(<Sidebar />);

    const dashboardLink = screen.getByRole("link", { name: "Dashboard" });
    const harnessesLink = screen.getByRole("link", { name: "Harnesses" });
    const settingsLink = screen.getByRole("link", { name: "Settings" });

    expect(dashboardLink).toHaveAttribute("href", "/dashboard");
    expect(harnessesLink).toHaveAttribute("href", "/harnesses");
    expect(settingsLink).toHaveAttribute("href", "/settings");
  });

  it("does not render legacy navigation items", () => {
    render(<Sidebar />);

    expect(screen.queryByText("Agents")).not.toBeInTheDocument();
    expect(screen.queryByText("Runs")).not.toBeInTheDocument();
    expect(screen.queryByText("Chat")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /agents/i })).not.toBeInTheDocument();
  });

  it("highlights the active navigation item", () => {
    mockPathname.mockReturnValue("/harnesses");
    render(<Sidebar />);

    const harnessesLink = screen.getByRole("link", { name: /harnesses/i });
    expect(harnessesLink).toHaveClass("bg-primary");
  });

  it("highlights navigation for nested routes", () => {
    mockPathname.mockReturnValue("/harnesses/123/sessions/456");
    render(<Sidebar />);

    const harnessesLink = screen.getByRole("link", { name: /harnesses/i });
    expect(harnessesLink).toHaveClass("bg-primary");
  });

  it("renders version in footer", () => {
    render(<Sidebar />);

    expect(screen.getByText("Everruns v0.1.0")).toBeInTheDocument();
  });

  it("has exactly 3 navigation items", () => {
    render(<Sidebar />);

    // Get nav links (excluding logo link)
    const navLinks = screen.getAllByRole("link").filter(
      link => link.getAttribute("href") !== "/dashboard" || link.textContent?.includes("Dashboard")
    );
    // Filter to only nav items (Dashboard, Harnesses, Settings)
    const navItems = ["Dashboard", "Harnesses", "Settings"];
    const foundNavLinks = navLinks.filter(link =>
      navItems.some(item => link.textContent?.includes(item))
    );
    expect(foundNavLinks).toHaveLength(3);
  });
});
