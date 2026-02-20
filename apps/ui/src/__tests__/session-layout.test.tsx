import { render, screen, act, waitFor } from "@testing-library/react";
import { Suspense } from "react";

// Mock next/navigation
const mockPathname = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
}));

// Mock next/link
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    href,
    className,
  }: {
    children: React.ReactNode;
    href: string;
    className?: string;
  }) => (
    <a href={href} className={className}>
      {children}
    </a>
  ),
}));

// Mock UI components
jest.mock("@/components/ui/button", () => ({
  buttonVariants: ({ variant }: { variant: string; size: string }) =>
    variant === "default" ? "btn-default" : "btn-ghost",
  Button: ({ children, ...props }: { children: React.ReactNode }) => (
    <button {...props}>{children}</button>
  ),
}));

jest.mock("@/components/ui/badge", () => ({
  Badge: ({
    children,
    variant,
    className,
  }: {
    children: React.ReactNode;
    variant?: string;
    className?: string;
  }) => (
    <span data-variant={variant} className={className}>
      {children}
    </span>
  ),
}));

jest.mock("@/components/ui/skeleton", () => ({
  Skeleton: ({ className }: { className?: string }) => (
    <div data-testid="skeleton" className={className} />
  ),
}));

jest.mock("@/components/ui/tooltip", () => ({
  TooltipProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

jest.mock("@/components/ui/input", () => ({
  Input: (props: Record<string, unknown>) => <input {...props} />,
}));

jest.mock("@/components/ui/copy-button", () => ({
  CopyButton: () => <button aria-label="Copy" />,
}));

jest.mock("@/lib/utils", () => ({
  cn: (...args: unknown[]) => args.filter(Boolean).join(" "),
  shortenId: (id: string) => id.slice(0, 8),
}));

// Mock session hooks
jest.mock("@/hooks/use-sessions", () => ({
  useUpdateSession: () => ({
    mutate: jest.fn(),
    isPending: false,
  }),
}));

// Mock the SessionProvider to skip data fetching
const mockSessionContext = {
  agent: { name: "Test Agent", id: "agent-123" } as Record<string, unknown>,
  session: {
    id: "ses-abc12345",
    title: "Test Session",
    agent_id: "agent-123",
    status: "idle",
    created_at: "2025-01-01T00:00:00Z",
    active_schedule_count: 0,
  } as Record<string, unknown> | null,
  llmModel: { display_name: "GPT-4" } as Record<string, unknown> | null,
  sessionLoading: false,
  effectiveStatus: "idle" as string | undefined,
  liveUsage: null as Record<string, unknown> | null,
  agentId: "agent-123",
};

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useSessionContext: () => mockSessionContext,
}));

// Import after mocks
import SessionLayout from "@/app/(main)/sessions/[sessionId]/layout";

describe("SessionLayout", () => {
  beforeEach(() => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/chat");
    mockSessionContext.sessionLoading = false;
    mockSessionContext.effectiveStatus = "idle";
    mockSessionContext.liveUsage = null;
    mockSessionContext.llmModel = { display_name: "GPT-4" };
    mockSessionContext.agent = { name: "Test Agent", id: "agent-123" };
    mockSessionContext.session = {
      id: "ses-abc12345",
      title: "Test Session",
      agent_id: "agent-123",
      status: "idle",
      created_at: "2025-01-01T00:00:00Z",
      active_schedule_count: 0,
    };
  });

  async function renderLayout(children?: React.ReactNode) {
    let result: ReturnType<typeof render> | undefined;
    await act(async () => {
      result = render(
        <Suspense fallback={<div data-testid="suspense-fallback">Loading...</div>}>
          <SessionLayout params={Promise.resolve({ sessionId: "ses-abc12345" })}>
            {children ?? <div data-testid="child-content">Chat Content</div>}
          </SessionLayout>
        </Suspense>,
      );
    });
    return result!;
  }

  it("uses h-full for proper height (not calc-based)", async () => {
    const { container } = await renderLayout();

    await waitFor(() => {
      // The main wrapper div should use h-full, not h-[calc(100vh-4rem)]
      const layoutDiv = container.querySelector('[class*="h-full"]');
      expect(layoutDiv).toBeInTheDocument();
    });

    // Ensure the old broken calc-based height is NOT present
    const calcDiv = container.querySelector('[class*="calc"]');
    expect(calcDiv).not.toBeInTheDocument();
  });

  it("renders children content", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByTestId("child-content")).toBeInTheDocument();
    });
    expect(screen.getByText("Chat Content")).toBeInTheDocument();
  });

  it("renders session title", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByText("Test Session")).toBeInTheDocument();
    });
  });

  it("renders all tab navigation links", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /chat/i })).toBeInTheDocument();
    });
    expect(screen.getByRole("link", { name: /workspace/i })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /storage/i })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /events/i })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /trajectory/i })).toBeInTheDocument();
  });

  it("renders correct tab hrefs", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /chat/i })).toHaveAttribute(
        "href",
        "/sessions/ses-abc12345/chat",
      );
    });
    expect(screen.getByRole("link", { name: /workspace/i })).toHaveAttribute(
      "href",
      "/sessions/ses-abc12345/files",
    );
    expect(screen.getByRole("link", { name: /storage/i })).toHaveAttribute(
      "href",
      "/sessions/ses-abc12345/storage",
    );
    expect(screen.getByRole("link", { name: /events/i })).toHaveAttribute(
      "href",
      "/sessions/ses-abc12345/events",
    );
    expect(screen.getByRole("link", { name: /trajectory/i })).toHaveAttribute(
      "href",
      "/sessions/ses-abc12345/trajectory",
    );
  });

  it("highlights active chat tab", async () => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/chat");
    await renderLayout();

    await waitFor(() => {
      const chatLink = screen.getByRole("link", { name: /chat/i });
      expect(chatLink).toHaveClass("btn-default");
    });
  });

  it("highlights active files tab", async () => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/files");
    await renderLayout();

    await waitFor(() => {
      const filesLink = screen.getByRole("link", { name: /workspace/i });
      expect(filesLink).toHaveClass("btn-default");
    });
  });

  it("shows inactive style for non-active tabs", async () => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/chat");
    await renderLayout();

    await waitFor(() => {
      const eventsLink = screen.getByRole("link", { name: /events/i });
      expect(eventsLink).toHaveClass("btn-ghost");
    });
  });

  it("renders Back to Sessions link", async () => {
    await renderLayout();

    await waitFor(() => {
      const backLink = screen.getByRole("link", { name: /back to sessions/i });
      expect(backLink).toHaveAttribute("href", "/sessions");
    });
  });

  it("renders agent name with link", async () => {
    await renderLayout();

    await waitFor(() => {
      const agentLink = screen.getByRole("link", { name: /test agent/i });
      expect(agentLink).toHaveAttribute("href", "/agents/agent-123");
    });
  });

  it("renders Ready badge for idle session", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByText("Ready")).toBeInTheDocument();
    });
  });

  it("renders LLM model badge", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByText("GPT-4")).toBeInTheDocument();
    });
  });

  it("shows loading skeletons when session is loading", async () => {
    mockSessionContext.sessionLoading = true;
    await renderLayout();

    await waitFor(() => {
      const skeletons = screen.getAllByTestId("skeleton");
      expect(skeletons.length).toBeGreaterThan(0);
    });
  });

  it("shows error state when session is not found", async () => {
    mockSessionContext.sessionLoading = false;
    mockSessionContext.session = null;
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByText("Session not found")).toBeInTheDocument();
    });
  });

  it("has flex-col layout for proper content stacking", async () => {
    const { container } = await renderLayout();

    await waitFor(() => {
      const layoutDiv = container.querySelector('[class*="flex"][class*="flex-col"][class*="h-full"]');
      expect(layoutDiv).toBeInTheDocument();
    });
  });
});
