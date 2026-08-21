import { act, render, screen, waitFor } from "@testing-library/react";
import { WebMcpProvider, getEverrunsPageContext } from "@/providers/webmcp-provider";
import { useWebMcp } from "@/providers/webmcp-context";
import type { WebMcpToolDefinition } from "@/lib/webmcp/types";

const mockPush = jest.fn();
const mockUseFeatureFlag = jest.fn(() => true);
let pathname = "/dashboard";

jest.mock("next/navigation", () => ({
  usePathname: () => pathname,
  useRouter: () => ({ push: mockPush }),
}));

jest.mock("@/providers/auth-provider", () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: "user-1" } }),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({
    currentOrg: { public_id: "org_00000000000000000000000000000001", name: "Default" },
  }),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => mockUseFeatureFlag(),
}));

jest.mock("@/lib/api/agents", () => ({
  searchAgents: jest.fn().mockResolvedValue({ data: [], total: 0, offset: 0, limit: 3 }),
  getAgent: jest.fn().mockResolvedValue({ id: "agent_00000000000000000000000000000001" }),
}));

jest.mock("@/lib/api/sessions", () => ({
  listSessions: jest.fn().mockResolvedValue({ data: [], total: 0, offset: 0, limit: 3 }),
  getSession: jest.fn().mockResolvedValue({ id: "session_00000000000000000000000000000001" }),
}));

describe("WebMcpProvider", () => {
  let tools: Map<string, WebMcpToolDefinition>;

  beforeEach(() => {
    jest.clearAllMocks();
    pathname = "/dashboard";
    mockUseFeatureFlag.mockReturnValue(true);
    tools = new Map();
    Object.defineProperty(document, "modelContext", {
      configurable: true,
      value: Object.assign(new EventTarget(), {
        registerTool: jest.fn(
          async (definition: WebMcpToolDefinition, options?: { signal?: AbortSignal }) => {
            tools.set(definition.name, definition);
            options?.signal?.addEventListener("abort", () => tools.delete(definition.name), {
              once: true,
            });
          },
        ),
      }),
    });
  });

  afterEach(() => {
    Reflect.deleteProperty(document, "modelContext");
  });

  it("registers shell tools and aborts them on unmount", async () => {
    const view = render(
      <WebMcpProvider>
        <div>child</div>
      </WebMcpProvider>,
    );

    await waitFor(() => expect(tools.size).toBe(3));
    expect([...tools.keys()].sort()).toEqual([
      "everruns_get_context",
      "everruns_open",
      "everruns_search",
    ]);

    await expect(tools.get("everruns_get_context")?.execute({})).resolves.toEqual({
      organization_id: "org_00000000000000000000000000000001",
      organization_name: "Default",
      page: "dashboard",
    });

    view.unmount();
    expect(tools.size).toBe(0);
  });

  it("shows the whole approval description in a bounded, scrollable region", async () => {
    const description = `Send to this session: “${"x".repeat(8_000)}”`;

    function RequestApproval() {
      const webmcp = useWebMcp();
      return (
        <button
          type="button"
          onClick={() => {
            void webmcp
              .requestApproval({
                title: "Send this agent message?",
                description,
                confirmLabel: "Send message",
              })
              .catch(() => {});
          }}
        >
          ask
        </button>
      );
    }

    render(
      <WebMcpProvider>
        <RequestApproval />
      </WebMcpProvider>,
    );
    act(() => screen.getByRole("button", { name: "ask" }).click());

    const rendered = await screen.findByText(description);
    // Nothing is elided: the user approves exactly the text that gets sent.
    expect(rendered).toHaveTextContent(description, { normalizeWhitespace: false });
    // And the dialog stays usable: the description scrolls inside a bounded
    // box instead of pushing the decision buttons out of the viewport.
    expect(rendered).toHaveClass("max-h-[40svh]", "overflow-y-auto");
    expect(screen.getByRole("button", { name: "Reject" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeInTheDocument();
  });

  it("does not register tools when the feature is disabled", async () => {
    mockUseFeatureFlag.mockReturnValue(false);
    render(
      <WebMcpProvider>
        <div>child</div>
      </WebMcpProvider>,
    );

    await Promise.resolve();
    expect(tools.size).toBe(0);
  });
});

describe("getEverrunsPageContext", () => {
  it("binds supported resource routes", () => {
    expect(getEverrunsPageContext("/agents/agent_123")).toEqual({
      page: "agent",
      resource_type: "agent",
      resource_id: "agent_123",
    });
    expect(getEverrunsPageContext("/sessions/session_123/chat")).toEqual({
      page: "session",
      resource_type: "session",
      resource_id: "session_123",
    });
  });
});
