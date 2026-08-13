import { render, screen, act, waitFor, fireEvent, within } from "@testing-library/react";
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

// Mock session hooks. The header's only mutation is fork, which creates a new
// session rather than changing this one.
const mockForkMutate = jest.fn();
jest.mock("@/hooks/use-sessions", () => ({
  useForkSession: () => ({ mutate: mockForkMutate, isPending: false }),
}));

jest.mock("@/lib/session-export", () => ({
  downloadSessionExport: jest.fn(),
}));

jest.mock("@/providers/notifications-provider", () => ({
  useOptionalNotificationsContext: () => undefined,
}));

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ locale: "en", t: (key: string) => key }),
}));

// Mock providers hook (used by the layout to resolve the session trace link).
// Avoids requiring OrgProvider in this isolated layout render.
jest.mock("@/hooks/use-providers", () => ({
  useProviders: () => ({ data: [] }),
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
  llmModelLoading: false,
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
    mockPathname.mockReturnValue("/sessions/ses-abc12345/transcript");
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
      // The title also appears as the breadcrumb's trailing segment (EVE-869),
      // so assert the masthead heading specifically.
      expect(screen.getByRole("heading", { name: "Test Session" })).toBeInTheDocument();
    });
  });

  it("renders the recording tabs in information-architecture order", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /transcript/i })).toBeInTheDocument();
    });
    expect(
      screen
        .getAllByRole("link")
        .filter((link) =>
          ["Transcript", "Timeline", "Work", "Events", "Workspace", "Cost"].includes(
            link.textContent ?? "",
          ),
        )
        .map((link) => link.textContent),
    ).toEqual(["Transcript", "Timeline", "Work", "Events", "Workspace", "Cost"]);
  });

  it("points each tab at its route", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /transcript/i })).toHaveAttribute(
        "href",
        "/sessions/ses-abc12345/transcript",
      );
      expect(screen.getByRole("link", { name: /timeline/i })).toHaveAttribute(
        "href",
        "/sessions/ses-abc12345/timeline",
      );
    });
    expect(screen.getByRole("link", { name: /workspace/i })).toHaveAttribute(
      "href",
      "/sessions/ses-abc12345/files",
    );
    expect(screen.getByRole("link", { name: /cost/i })).toHaveAttribute(
      "href",
      "/sessions/ses-abc12345/cost",
    );
  });

  it("offers no chat, storage, schedules or trajectory tab", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /timeline/i })).toBeInTheDocument();
    });
    for (const retired of [/^chat$/i, /^storage$/i, /^schedules$/i, /^trajectory$/i]) {
      expect(screen.queryByRole("link", { name: retired })).not.toBeInTheDocument();
    }
  });

  it("highlights the active tab", async () => {
    mockPathname.mockReturnValue("/sessions/ses-abc12345/files");
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /workspace/i })).toHaveClass("border-primary");
    });
    expect(screen.getByRole("link", { name: /timeline/i })).not.toHaveClass("border-primary");
  });

  it("highlights Transcript and uses it in the page title", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /transcript/i })).toHaveClass("border-primary");
      expect(document.title).toContain("Transcript · Test Session · Session");
    });
  });

  it("offers the three escape hatches and no session-editing control", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /fork into chat/i })).toBeInTheDocument();
    });
    expect(screen.getByRole("link", { name: /open agent/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /export session/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /edit session title/i })).not.toBeInTheDocument();
  });

  it("forks into a new session rather than mutating this one", async () => {
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /fork into chat/i })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: /fork into chat/i }));

    expect(mockForkMutate).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "ses-abc12345" }),
      expect.anything(),
    );
  });

  it("names the owning group and keeps a link back to the sessions list", async () => {
    await renderLayout();

    await waitFor(() => {
      // EVE-869: the breadcrumb replaced the standalone back link, so it has to
      // stay navigable to the list — losing the way back is the failure mode.
      const backLink = screen.getByRole("link", { name: "Sessions" });
      expect(backLink).toHaveAttribute("href", "/sessions");
    });

    // The group is derived from the sidebar section that owns /sessions, and is
    // a label rather than a link — there is no group index page to point at.
    const breadcrumb = screen.getByRole("navigation", { name: "Breadcrumb" });
    expect(within(breadcrumb).getByText("Operational")).toBeInTheDocument();
    expect(within(breadcrumb).queryByRole("link", { name: "Operational" })).toBeNull();
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
      expect(screen.getByRole("link", { name: /transcript/i })).toBeInTheDocument();
    });
    expect(screen.getByRole("link", { name: /timeline/i })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /events/i })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /cost/i })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /workspace/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /^work$/i })).not.toBeInTheDocument();
  });

  it("shows Workspace only when the file_system feature is present", async () => {
    mockSessionContext.session = {
      ...mockSessionContext.session,
      features: ["file_system"],
    };
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /workspace/i })).toBeInTheDocument();
    });
    expect(screen.queryByRole("link", { name: /^work$/i })).not.toBeInTheDocument();
  });

  it("shows Work when the session scheduled or leased anything", async () => {
    mockSessionContext.session = {
      ...mockSessionContext.session,
      active_schedule_count: 3,
      features: ["schedules"],
    };
    await renderLayout();

    await waitFor(() => {
      expect(screen.getByRole("link", { name: /work/i })).toBeInTheDocument();
    });
    expect(screen.getByText("3")).toBeInTheDocument();
  });
});
