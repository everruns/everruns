// Regression test for EVE-785: the empty dashboard renders three links to
// `/agents/new` (Active Agents header, empty state, Quick Actions). With
// Next.js default viewport prefetch these eagerly fetched the agent-creation
// RSC payload on a cold production load, before any user intent. The links must
// disable automatic prefetch and prefetch only on hover/focus intent — the same
// policy EVE-780 applied to the sidebar — while still navigating on click.

import { render, screen, fireEvent } from "@testing-library/react";
import { AgentListWidget } from "@/components/dashboard/agent-list-widget";
import { NewAgentLink } from "@/components/dashboard/new-agent-link";

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

const mockPrefetch = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({
    push: jest.fn(),
    prefetch: mockPrefetch,
    replace: jest.fn(),
    back: jest.fn(),
  }),
}));

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ locale: "en" }),
}));

beforeEach(() => {
  mockPrefetch.mockClear();
});

describe("dashboard /agents/new prefetch (EVE-785)", () => {
  it("empty widget links to /agents/new with automatic prefetch disabled", () => {
    render(<AgentListWidget agents={[]} />);

    const links = screen
      .getAllByRole("link")
      .filter((link) => link.getAttribute("href") === "/agents/new");

    // Active Agents header + empty-state call-to-action.
    expect(links).toHaveLength(2);
    for (const link of links) {
      expect(link).toHaveAttribute("data-prefetch", "false");
    }
  });

  it("does not prefetch /agents/new on mount, only on hover/focus intent", () => {
    render(<AgentListWidget agents={[]} />);

    // No automatic prefetch fires from rendering the links.
    expect(mockPrefetch).not.toHaveBeenCalled();

    const link = screen
      .getAllByRole("link")
      .find((el) => el.getAttribute("href") === "/agents/new")!;

    fireEvent.mouseEnter(link);
    expect(mockPrefetch).toHaveBeenCalledWith("/agents/new");

    mockPrefetch.mockClear();
    fireEvent.focus(link);
    expect(mockPrefetch).toHaveBeenCalledWith("/agents/new");

    // The intent prefetch only ever targets the single agent-creation route,
    // so repeated links never fan out to other destinations.
    for (const call of mockPrefetch.mock.calls) {
      expect(call).toEqual(["/agents/new"]);
    }
  });

  it("NewAgentLink preserves click navigation to /agents/new", () => {
    render(<NewAgentLink>Create</NewAgentLink>);

    const link = screen.getByRole("link", { name: "Create" });
    expect(link).toHaveAttribute("href", "/agents/new");
    expect(link).toHaveAttribute("data-prefetch", "false");
  });
});
