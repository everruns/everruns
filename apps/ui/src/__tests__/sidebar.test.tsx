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
    expect(screen.getByText("Agents")).toBeInTheDocument();
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("renders correct navigation links", () => {
    render(<Sidebar />);

    const dashboardLink = screen.getByRole("link", { name: "Dashboard" });
    const agentsLink = screen.getByRole("link", { name: "Agents" });
    const settingsLink = screen.getByRole("link", { name: "Settings" });

    expect(dashboardLink).toHaveAttribute("href", "/dashboard");
    expect(agentsLink).toHaveAttribute("href", "/agents");
    expect(settingsLink).toHaveAttribute("href", "/settings");
  });

  it("does not render legacy navigation items", () => {
    render(<Sidebar />);

    expect(screen.queryByText("Harnesses")).not.toBeInTheDocument();
    expect(screen.queryByText("Runs")).not.toBeInTheDocument();
    expect(screen.queryByText("Chat")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /harnesses/i })).not.toBeInTheDocument();
  });

  it("highlights the active navigation item", () => {
    mockPathname.mockReturnValue("/agents");
    render(<Sidebar />);

    const agentsLink = screen.getByRole("link", { name: /agents/i });
    expect(agentsLink).toHaveClass("bg-primary");
  });

  it("highlights navigation for nested routes", () => {
    mockPathname.mockReturnValue("/agents/123/sessions/456");
    render(<Sidebar />);

    const agentsLink = screen.getByRole("link", { name: /agents/i });
    expect(agentsLink).toHaveClass("bg-primary");
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
    // Filter to only nav items (Dashboard, Agents, Settings)
    const navItems = ["Dashboard", "Agents", "Settings"];
    const foundNavLinks = navLinks.filter(link =>
      navItems.some(item => link.textContent?.includes(item))
    );
    expect(foundNavLinks).toHaveLength(3);
  });
});
