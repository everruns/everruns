import { render, screen } from "@testing-library/react";
import { SandboxFleetTab } from "@/app/(main)/sandboxes/(tabs)/fleet-tab";
import type { SandboxFleetStats } from "@/lib/api/sandboxes";

let mockStats: Partial<SandboxFleetStats> = {};

jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: jest.fn(), push: jest.fn() }),
  useSearchParams: () => new URLSearchParams(),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));
jest.mock("@/components/chat-mode/page-view-state", () => ({ useChatPageFilters: jest.fn() }));
jest.mock("@/hooks/use-sandbox-templates", () => ({
  useSandboxTargets: () => ({ data: { items: [] } }),
}));
jest.mock("@/components/sandboxes/sandbox-detail-drawer", () => ({
  SandboxDetailDrawer: () => null,
}));
jest.mock("@/hooks/use-sandbox-fleet", () => ({
  useSandboxFleet: () => ({ data: { items: [], total: 0 }, isLoading: false }),
  useSandboxFleetStats: () => ({ data: { ...baseStats(), ...mockStats } }),
  useSandboxTimeline: () => ({ data: undefined, isLoading: false }),
}));

function baseStats(): SandboxFleetStats {
  return {
    by_state: [],
    live_by_provider: [],
    window_days: 7,
    created_in_window: 0,
    created_in_prior_window: 0,
    running_seconds_in_window: 0,
    recoveries_in_window: 0,
    needs_attention: 0,
  } as unknown as SandboxFleetStats;
}

describe("SandboxFleetTab", () => {
  it("teaches instead of showing zero tiles when the org has no Sandboxes", () => {
    mockStats = {};
    render(<SandboxFleetTab />);

    expect(screen.getByText("No Sandboxes yet")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Set up a Sandbox Template" })).toHaveAttribute(
      "href",
      "/sandboxes/templates",
    );
    expect(screen.queryByText("Live now")).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText("Search sessions, agents, provider ids")).toBeNull();
  });

  it("says the filters hide Sandboxes when the org has some", () => {
    mockStats = { by_state: [{ key: "deleted", count: 3 }] } as Partial<SandboxFleetStats>;
    render(<SandboxFleetTab />);

    expect(screen.getByText("Live now")).toBeInTheDocument();
    expect(screen.getByText("No Sandboxes match these filters")).toBeInTheDocument();
  });
});
