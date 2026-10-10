// Regression test for EVE-793: links to `/agents/new` eagerly fetched the
// agent-creation RSC payload through Next.js viewport prefetch before any user
// intent. Every such link goes through `NewAgentLink`, which disables automatic
// prefetch and prefetches only on hover/focus intent, while still navigating
// on click.

import { render, screen, fireEvent } from "@testing-library/react";
import { NewAgentLink } from "@/components/agents/new-agent-link";

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
  useRouter: () => ({ push: jest.fn(), prefetch: mockPrefetch }),
}));

beforeEach(() => {
  mockPrefetch.mockClear();
});

describe("NewAgentLink prefetch (EVE-793)", () => {
  it("disables automatic prefetch", () => {
    render(<NewAgentLink>New agent</NewAgentLink>);

    const link = screen.getByRole("link", { name: "New agent" });
    expect(link).toHaveAttribute("href", "/agents/new");
    expect(link).toHaveAttribute("data-prefetch", "false");
  });

  it("does not prefetch on mount, only on hover/focus intent", () => {
    render(<NewAgentLink>New agent</NewAgentLink>);
    expect(mockPrefetch).not.toHaveBeenCalled();

    const link = screen.getByRole("link", { name: "New agent" });
    fireEvent.mouseEnter(link);
    expect(mockPrefetch).toHaveBeenCalledWith("/agents/new");

    mockPrefetch.mockClear();
    fireEvent.focus(link);
    expect(mockPrefetch).toHaveBeenCalledWith("/agents/new");
  });
});
