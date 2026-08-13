// Regression test for EVE-793: a cold `/agents` load rendered two links to
// `/agents/new` (the masthead "New agent" action and the empty-state "Create
// your first agent" call-to-action). With Next.js default viewport prefetch
// these eagerly fetched the agent-creation RSC payload twice before any user
// intent. Both links must disable automatic prefetch and prefetch only on
// hover/focus intent — the same policy EVE-785 applied to the dashboard — while
// still navigating on click.

import { render, screen, fireEvent } from "@testing-library/react";
import AgentsPage from "@/app/(main)/agents/page";

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
  usePathname: () => "/agents",
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

// The agent/example cards transitively import an ESM-only markdown renderer that
// Jest does not transform. They never render in this empty-state test, so stub
// them out to keep the module graph light and the test deterministic.
jest.mock("@/components/agents", () => ({
  AgentCard: () => null,
  ExampleCard: () => null,
}));

// Empty agent set so the primary frame renders the empty-state CTA, and the
// examples section stays quiet. Everything else is inert.
jest.mock("@/hooks", () => ({
  usePageTitle: () => {},
  useAgents: () => ({ data: [], isLoading: false, error: null }),
  useCapabilities: () => ({ data: [] }),
  useAgentExamples: () => ({ data: [], isLoading: false, error: null }),
  useImportAgent: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useImportAgentExample: () => ({ mutateAsync: jest.fn(), isPending: false }),
}));

beforeEach(() => {
  mockPrefetch.mockClear();
});

describe("agents page /agents/new prefetch (EVE-793)", () => {
  it("both /agents/new links disable automatic prefetch", () => {
    render(<AgentsPage />);

    const links = screen
      .getAllByRole("link")
      .filter((link) => link.getAttribute("href") === "/agents/new");

    // Masthead "New agent" + empty-state "Create your first agent".
    expect(links).toHaveLength(2);
    for (const link of links) {
      expect(link).toHaveAttribute("data-prefetch", "false");
    }
  });

  it("does not prefetch /agents/new on mount, only on hover/focus intent", () => {
    render(<AgentsPage />);

    // Nothing prefetches from simply rendering the links.
    expect(mockPrefetch).not.toHaveBeenCalled();

    const links = screen
      .getAllByRole("link")
      .filter((link) => link.getAttribute("href") === "/agents/new");

    fireEvent.mouseEnter(links[0]);
    expect(mockPrefetch).toHaveBeenCalledWith("/agents/new");

    mockPrefetch.mockClear();
    fireEvent.focus(links[1]);
    expect(mockPrefetch).toHaveBeenCalledWith("/agents/new");

    // The intent prefetch only ever targets the single agent-creation route, so
    // repeated links never fan out to other destinations.
    for (const call of mockPrefetch.mock.calls) {
      expect(call).toEqual(["/agents/new"]);
    }
  });
});
