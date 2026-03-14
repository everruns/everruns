import { render, screen, act, waitFor, fireEvent } from "@testing-library/react";
import { Suspense } from "react";

// Mock next/navigation
const mockPathname = jest.fn();
const mockPush = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => mockPathname(),
  useRouter: () => ({ push: mockPush }),
}));

// Mock next/link
jest.mock("next/link", () => ({
  __esModule: true,
  // eslint-disable-next-line nextjs/no-html-link-for-pages
  default: ({
    children,
    href,
    className,
  }: {
    children: React.ReactNode;
    href: string;
    className?: string;
  }) => (
    // eslint-disable-next-line nextjs/no-html-link-for-pages
    <a href={href} className={className}>
      {children}
    </a>
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

jest.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DropdownMenuTrigger: ({
    children,
    className,
    ...props
  }: {
    children: React.ReactNode;
    className?: string;
  }) => (
    <button className={className} {...props}>
      {children}
    </button>
  ),
  DropdownMenuPositioner: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DropdownMenuContent: ({
    children,
    className,
  }: {
    children: React.ReactNode;
    className?: string;
  }) => <div className={className}>{children}</div>,
  DropdownMenuGroup: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DropdownMenuLabel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DropdownMenuItem: ({
    children,
    className,
    onClick,
  }: {
    children: React.ReactNode;
    className?: string;
    onClick?: () => void;
  }) => (
    <button className={className} onClick={onClick}>
      {children}
    </button>
  ),
  DropdownMenuShortcut: ({
    children,
    className,
  }: {
    children: React.ReactNode;
    className?: string;
  }) => <span className={className}>{children}</span>,
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
  agent: { name: "Test Agent", id: "agent-123", status: "active" } as Record<string, unknown>,
  session: {
    id: "ses-abc12345",
    title: "Test Session",
    agent_id: "agent-123",
    status: "idle",
    created_at: "2025-01-01T00:00:00Z",
    active_schedule_count: 0,
    features: ["file_system", "secrets", "key_value", "schedules"],
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
    mockPush.mockReset();
    mockPathname.mockReturnValue("/sessions/ses-abc12345/chat");
    mockSessionContext.sessionLoading = false;
    mockSessionContext.effectiveStatus = "idle";
    mockSessionContext.liveUsage = null;
    mockSessionContext.llmModel = { display_name: "GPT-4" };
    mockSessionContext.agent = { name: "Test Agent", id: "agent-123", status: "active" };
    mockSessionContext.session = {
      id: "ses-abc12345",
      title: "Test Session",
      agent_id: "agent-123",
      status: "idle",
      created_at: "2025-01-01T00:00:00Z",
      active_schedule_count: 0,
      features: ["file_system", "secrets", "key_value", "schedules"],
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

  it("renders compact navigation controls", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /chat/i })).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: /advanced navigation/i })).toBeInTheDocument();
  });

  it("renders correct compact navigation destinations", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /chat/i })).toHaveAttribute(
        "href",
        "/sessions/ses-abc12345/chat",
      );
    });

    fireEvent.click(screen.getByRole("button", { name: /advanced navigation/i }));
    fireEvent.click(screen.getByRole("button", { name: /workspace/i }));

    expect(mockPush).toHaveBeenCalledWith("/sessions/ses-abc12345/files");
  });

  it("highlights active chat navigation", async () => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/chat");
    await renderLayout();

    await waitFor(() => {
      const chatLink = screen.getByRole("link", { name: /chat/i });
      expect(chatLink).toHaveClass("border-primary");
      expect(chatLink).toHaveClass("bg-card");
    });
  });

  it("highlights advanced navigation when an advanced view is active", async () => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/files");
    await renderLayout();

    await waitFor(() => {
      const advancedTrigger = screen.getByRole("button", { name: /advanced navigation/i });
      expect(advancedTrigger).toHaveClass("border-primary");
      expect(advancedTrigger).toHaveClass("bg-card");
    });
  });

  it("shows inactive style for advanced navigation on chat", async () => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/chat");
    await renderLayout();

    await waitFor(() => {
      const advancedTrigger = screen.getByRole("button", { name: /advanced navigation/i });
      expect(advancedTrigger).not.toHaveClass("border-primary");
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

  it("hides feature-gated tabs when features are empty", async () => {
    mockSessionContext.session = {
      ...mockSessionContext.session,
      features: [],
    };
    await renderLayout();

    await waitFor(() => {
      // Chat and the advanced menu remain available
      expect(screen.getByRole("link", { name: /chat/i })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /advanced navigation/i })).toBeInTheDocument();
    });

    // Feature-gated advanced items should NOT be present
    expect(screen.queryByRole("button", { name: /workspace/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /storage/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /schedules/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /events/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /trajectory/i })).toBeInTheDocument();
  });

  it("shows only workspace tab when only file_system feature is present", async () => {
    mockSessionContext.session = {
      ...mockSessionContext.session,
      features: ["file_system"],
    };
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /workspace/i })).toBeInTheDocument();
    });

    expect(screen.queryByRole("button", { name: /storage/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /schedules/i })).not.toBeInTheDocument();
  });

  it("shows schedules tab when schedules feature is present", async () => {
    mockSessionContext.session = {
      ...mockSessionContext.session,
      features: ["schedules"],
    };
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /schedules/i })).toBeInTheDocument();
    });

    expect(screen.queryByRole("button", { name: /workspace/i })).not.toBeInTheDocument();
  });

  it("shows schedules count inside advanced navigation", async () => {
    mockSessionContext.session = {
      ...mockSessionContext.session,
      active_schedule_count: 3,
      features: ["schedules"],
    };
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /schedules/i })).toBeInTheDocument();
    });
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("has flex-col layout for proper content stacking", async () => {
    const { container } = await renderLayout();

    await waitFor(() => {
      const layoutDiv = container.querySelector(
        '[class*="flex"][class*="flex-col"][class*="h-full"]',
      );
      expect(layoutDiv).toBeInTheDocument();
    });
  });
});
