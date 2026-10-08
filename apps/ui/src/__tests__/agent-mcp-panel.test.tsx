import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AgentMcpPanel } from "@/components/agents/agent-mcp-panel";
import type { Agent, AgentMcpAttachment } from "@/lib/api/types";

const mockRefetch = jest.fn();
const mockUpdateAgent = jest.fn();
const mockRevokeConnection = jest.fn();
const mockUseAgentMcpAttachments = jest.fn();
const mockUseMcpServers = jest.fn();

jest.mock("@/hooks/use-agents", () => ({
  useAgentMcpAttachments: () => mockUseAgentMcpAttachments(),
  useUpdateAgent: () => ({
    mutateAsync: mockUpdateAgent,
    isPending: false,
  }),
  useRevokeAgentMcpConnection: () => ({
    mutate: mockRevokeConnection,
    isPending: false,
  }),
}));

jest.mock("@/hooks/use-mcp-servers", () => ({
  useMcpServers: () => mockUseMcpServers(),
}));

const mockUseUserMcpServers = jest.fn();
jest.mock("@/hooks/use-user-mcp-servers", () => ({
  useUserMcpServers: () => mockUseUserMcpServers(),
}));

const agent = {
  id: "agent-1",
  name: "research-agent",
  display_name: "Research Agent",
  description: null,
  system_prompt: "Research.",
  harness_id: "harness-1",
  default_model_id: null,
  tags: [],
  capabilities: [],
  mcpServers: {
    existing: {
      type: "http",
      url: "https://existing.example/mcp",
      actsAs: "none",
    },
  },
  status: "active",
  created_at: "2026-09-18T00:00:00Z",
  updated_at: "2026-09-18T00:00:00Z",
  archived_at: null,
  deleted_at: null,
} as Agent;

const attachment: AgentMcpAttachment = {
  name: "github",
  source: "agent",
  source_label: "This agent",
  contributor: null,
  overridden_sources: [
    { source: "harness", source_label: "Harness: researcher" },
    { source: "capability", source_label: "Capability: web search" },
  ],
  acts_as: "user",
  connect_in_chat: "ask",
  deferred: false,
  preset_name: "github",
  preset_id: "preset-1",
  connection_provider: "github",
  url: "https://api.githubcopilot.com/mcp",
  header_names: ["X-Organization"],
  tools_available: true,
  tools: ["search_repositories", "get_file_contents"],
  state: "ready",
  action: "none",
  connected_as: "octocat",
  can_revoke: true,
  editable: true,
};

function showAttachments(...attachments: AgentMcpAttachment[]) {
  mockUseAgentMcpAttachments.mockReturnValue({
    data: attachments,
    isLoading: false,
    error: null,
    refetch: mockRefetch,
  });
}

describe("AgentMcpPanel", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockRefetch.mockResolvedValue({});
    mockUpdateAgent.mockResolvedValue({});
    showAttachments(attachment);
    mockUseUserMcpServers.mockReturnValue({ data: [] });
    mockUseMcpServers.mockReturnValue({
      data: [
        {
          id: "preset-1",
          name: "github",
          description: "GitHub MCP server",
          url: "https://api.githubcopilot.com/mcp",
          transport_type: "http",
          status: "active",
          auth_mode: "oauth",
          api_key_set: false,
          headers: {},
          created_at: "2026-09-18T00:00:00Z",
          updated_at: "2026-09-18T00:00:00Z",
          archived_at: null,
          deleted_at: null,
        },
        {
          id: "preset-2",
          name: "microsoft_learn",
          description: "Microsoft Learn documentation",
          url: "https://learn.microsoft.com/api/mcp",
          transport_type: "http",
          status: "active",
          auth_mode: "none",
          api_key_set: false,
          headers: {},
          created_at: "2026-09-18T00:00:00Z",
          updated_at: "2026-09-18T00:00:00Z",
          archived_at: null,
          deleted_at: null,
        },
      ],
      isLoading: false,
    });
  });

  it("hides the user servers group without the user_mcp capability", () => {
    render(<AgentMcpPanel agent={agent} />);

    expect(screen.queryByText("User servers of the person chatting")).not.toBeInTheDocument();
  });

  it("shows the user servers switches and reports the viewer's skipped servers", async () => {
    mockUseUserMcpServers.mockReturnValue({
      data: [
        { id: "mcp_1", name: "github", enabled: true },
        { id: "mcp_2", name: "linear", enabled: true },
      ],
    });
    const withUserMcp = {
      ...agent,
      capabilities: [{ ref: "user_mcp", config: { manage: true } }],
    } as Agent;

    render(<AgentMcpPanel agent={withUserMcp} />);

    expect(screen.getByText("User servers of the person chatting")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Use their servers" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByRole("switch", { name: "Let the agent manage them" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    const custom = screen.getByRole("switch", { name: "Allow servers outside the catalog" });
    expect(custom).toHaveAttribute("aria-checked", "false");

    const skipped = screen.getByRole("list", { name: "Skipped user servers" });
    expect(within(skipped).getByText(/name clash with agent server/)).toBeInTheDocument();
    expect(within(skipped).queryByText("linear")).not.toBeInTheDocument();

    fireEvent.click(custom);
    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          capabilities: [{ ref: "user_mcp", config: { manage: true, allow_custom_urls: true } }],
        },
      }),
    );
  });

  it("shows the effective source, identity, overrides, connection, and tools", () => {
    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("Source:").parentElement).toHaveTextContent("This agent");
    expect(screen.getByText("Invoking user")).toBeInTheDocument();
    expect(screen.getByText(/Overrides:/)).toHaveTextContent(
      "Harness: researcher, Capability: web search",
    );
    expect(screen.getByText("Connected as octocat")).toBeInTheDocument();
    expect(screen.queryByText("search_repositories")).not.toBeInTheDocument();

    const toolsButton = screen.getByRole("button", { name: /Tools 2/ });
    expect(toolsButton).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(toolsButton);

    expect(toolsButton).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("search_repositories")).toBeInTheDocument();
    expect(screen.getByText("get_file_contents")).toBeInTheDocument();
  });
  it("offers authorization for a missing service grant when allowed", () => {
    showAttachments({
      ...attachment,
      acts_as: "service",
      state: "connection_missing",
      action: "authorize",
      connected_as: null,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByRole("link", { name: "Authorize" })).toHaveAttribute(
      "href",
      expect.stringContaining("mode=identity"),
    );
    expect(screen.queryByText("Ask an admin")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Connect" })).not.toBeInTheDocument();
  });

  it("shows ask-an-admin without an unusable service action", () => {
    showAttachments({
      ...attachment,
      acts_as: "service",
      state: "connection_missing",
      action: "ask_admin",
      connected_as: null,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("Ask an admin")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Authorize" })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Connect" })).not.toBeInTheDocument();
  });

  it("does not offer connected service revocation to a caller without permission", () => {
    showAttachments({
      ...attachment,
      acts_as: "service",
      can_revoke: false,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("Connected as octocat")).toBeInTheDocument();
    expect(screen.getByText("Ask an admin")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Revoke" })).not.toBeInTheDocument();
  });

  it("offers connection for the invoking user's missing grant", () => {
    showAttachments({
      ...attachment,
      state: "connection_missing",
      action: "connect",
      connected_as: null,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByRole("link", { name: "Connect" })).toHaveAttribute(
      "href",
      expect.stringContaining("mode=user"),
    );
    expect(screen.queryByRole("link", { name: "Authorize" })).not.toBeInTheDocument();
  });

  it("labels user_or_service and asks a manager for the agent's fallback login", () => {
    showAttachments({
      ...attachment,
      acts_as: "user_or_service",
      state: "connection_missing",
      action: "authorize",
      connected_as: null,
      can_revoke: false,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(
      screen.getByText("Acts as: the user, or the agent when the user has not connected"),
    ).toBeInTheDocument();
    const href = screen.getByRole("link", { name: "Authorize" }).getAttribute("href") ?? "";
    expect(href).toContain("mode=identity");
    expect(href).toContain("agent_id=agent-1");
  });

  it("lets a user_or_service caller running on the agent's login connect their own", () => {
    showAttachments({
      ...attachment,
      acts_as: "user_or_service",
      state: "ready",
      action: "connect",
      connected_as: "agent-bot",
      can_revoke: false,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("Connected as agent-bot")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Connect your account" })).toHaveAttribute(
      "href",
      expect.stringContaining("mode=user"),
    );
    expect(screen.getByText("Ask an admin")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Revoke" })).not.toBeInTheDocument();
  });

  it("sends the agent login of a connection-backed preset to the agent's integrations", () => {
    showAttachments({
      ...attachment,
      acts_as: "service",
      state: "connection_missing",
      action: "authorize",
      connected_as: null,
      service_connection_provider: "github",
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByRole("link", { name: "Authorize" })).toHaveAttribute(
      "href",
      "/agents/agent-1?tab=integrations",
    );
  });

  it("keeps capability attachments read-only and links to the capability", () => {
    showAttachments({
      ...attachment,
      source: "capability",
      source_label: "Web Search",
      contributor: {
        id: "web_search",
        name: "Web Search",
        href: "/capabilities/web_search",
      },
      can_revoke: false,
      editable: false,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByRole("link", { name: "View Web Search" })).toHaveAttribute(
      "href",
      "/capabilities/web_search",
    );
    expect(screen.queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Revoke" })).not.toBeInTheDocument();
  });

  it("suppresses connection controls for a capability attachment with a missing grant", () => {
    showAttachments({
      ...attachment,
      source: "capability",
      source_label: "Web Search",
      contributor: {
        id: "web_search",
        name: "Web Search",
        href: "/capabilities/web_search",
      },
      state: "connection_missing",
      action: "none",
      connected_as: null,
      can_revoke: false,
      editable: false,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("Connection required.")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Connect" })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Authorize" })).not.toBeInTheDocument();
    expect(screen.queryByText("Ask an admin")).not.toBeInTheDocument();
  });

  it("shows a repair path for missing presets without hiding the row", () => {
    showAttachments({
      ...attachment,
      state: "preset_missing",
      action: "none",
      connected_as: null,
      tools_available: false,
      tools: [],
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("This preset is no longer available.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "View catalog" })).toHaveAttribute(
      "href",
      "/settings/mcp-catalog",
    );
    fireEvent.click(screen.getByRole("button", { name: /Tools 0 Unavailable/ }));
    expect(screen.getByText("No cached tools are available.")).toBeInTheDocument();
  });

  it("calls out attachment names with colliding tool prefixes", () => {
    showAttachments(
      { ...attachment, name: "github-tools" },
      { ...attachment, name: "github_tools" },
    );

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getAllByText("Tool prefix collides with another attachment name.")).toHaveLength(
      2,
    );
  });

  it("adds a selected preset with the selected identity mode", async () => {
    showAttachments({ ...attachment, name: "other" });
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /github GitHub MCP server/ }));
    fireEvent.click(within(dialog).getByRole("radio", { name: "Invoking user" }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            github: {
              use: "catalog:github",
              actsAs: "user",
            },
          },
        },
      }),
    );
    expect(mockRefetch).toHaveBeenCalled();
  });
  it("adds a preset acting as each user, or the agent as a fallback", async () => {
    showAttachments({ ...attachment, name: "other" });
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /github GitHub MCP server/ }));
    fireEvent.click(
      within(dialog).getByRole("radio", {
        name: "Each user, or the agent if they have not connected",
      }),
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            github: { use: "catalog:github", actsAs: "user_or_service" },
          },
        },
      }),
    );
  });

  it("adds a preset that never asks to connect in chat", async () => {
    showAttachments({ ...attachment, name: "other" });
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /github GitHub MCP server/ }));
    fireEvent.click(within(dialog).getByRole("radio", { name: "Invoking user" }));
    const ask = within(dialog).getByRole("checkbox", {
      name: "Ask to connect in chat when a sign-in is missing",
    });
    expect(ask).toBeChecked();
    fireEvent.click(ask);
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            github: { use: "catalog:github", actsAs: "user", connectInChat: "never" },
          },
        },
      }),
    );
  });

  it("toggles connect in chat on an attachment authored on the agent", async () => {
    const withGithub = {
      ...agent,
      mcpServers: {
        ...agent.mcpServers,
        github: { use: "catalog:github", actsAs: "user" },
      },
    } as Agent;
    const { rerender } = render(<AgentMcpPanel agent={withGithub} />);

    const toggle = screen.getByRole("switch", { name: "Ask to connect in chat" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            github: { use: "catalog:github", actsAs: "user", connectInChat: "never" },
          },
        },
      }),
    );

    // Turning it back on drops the field: `ask` is the default.
    showAttachments({ ...attachment, connect_in_chat: "never" });
    rerender(
      <AgentMcpPanel
        agent={
          {
            ...withGithub,
            mcpServers: {
              ...withGithub.mcpServers,
              github: { use: "catalog:github", actsAs: "user", connectInChat: "never" },
            },
          } as Agent
        }
      />,
    );
    expect(screen.getByText(/fails the call with a link to settings/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Ask to connect in chat" }));
    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenLastCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            github: { use: "catalog:github", actsAs: "user" },
          },
        },
      }),
    );
  });

  it("toggles loading tools on demand for an attachment authored on the agent", async () => {
    const withGithub = {
      ...agent,
      mcpServers: {
        ...agent.mcpServers,
        github: { use: "catalog:github", actsAs: "user" },
      },
    } as Agent;
    const { rerender } = render(<AgentMcpPanel agent={withGithub} />);

    const toggle = screen.getByRole("switch", { name: "Load tools on demand" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            github: { use: "catalog:github", actsAs: "user", deferred: true },
          },
        },
      }),
    );

    // Turning it off drops the field: listing at turn start is the default.
    showAttachments({ ...attachment, deferred: true });
    rerender(
      <AgentMcpPanel
        agent={
          {
            ...withGithub,
            mcpServers: {
              ...withGithub.mcpServers,
              github: { use: "catalog:github", actsAs: "user", deferred: true },
            },
          } as Agent
        }
      />,
    );
    expect(screen.getByText(/loads its tools through tool search/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Load tools on demand" }));
    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenLastCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            github: { use: "catalog:github", actsAs: "user" },
          },
        },
      }),
    );
  });

  it("labels a read-only attachment whose tools load on demand", () => {
    showAttachments({ ...attachment, source: "harness", editable: false, deferred: true });
    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("Tools load on demand")).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Load tools on demand" })).not.toBeInTheDocument();
  });

  it("labels a read-only attachment that connects in settings only", () => {
    showAttachments({
      ...attachment,
      source: "harness",
      editable: false,
      connect_in_chat: "never",
    });
    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByText("Connects in settings only")).toBeInTheDocument();
    expect(
      screen.queryByRole("switch", { name: "Ask to connect in chat" }),
    ).not.toBeInTheDocument();
  });

  it("uses no identity for a preset without OAuth support", async () => {
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(
      within(dialog).getByRole("button", {
        name: /microsoft_learn Microsoft Learn documentation/,
      }),
    );

    expect(within(dialog).getByRole("radio", { name: "No identity" })).toBeChecked();
    expect(within(dialog).getByRole("radio", { name: "Service identity" })).toBeDisabled();
    expect(within(dialog).getByRole("radio", { name: "Invoking user" })).toBeDisabled();
    expect(dialog).toHaveTextContent("This preset does not support OAuth identity grants.");
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            microsoft_learn: {
              use: "catalog:microsoft_learn",
              actsAs: "none",
            },
          },
        },
      }),
    );
  });

  it("adds a custom HTTP server with parsed headers and no identity grant", async () => {
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Custom" }));
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "internal-docs" },
    });
    fireEvent.change(within(dialog).getByLabelText("URL"), {
      target: { value: "https://docs.example/mcp" },
    });
    fireEvent.change(within(dialog).getByLabelText("Headers"), {
      target: { value: "Authorization: Bearer token:with-colon" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: {
          mcpServers: {
            ...agent.mcpServers,
            "internal-docs": {
              type: "http",
              url: "https://docs.example/mcp",
              headers: { Authorization: "Bearer token:with-colon" },
              actsAs: "none",
            },
          },
        },
      }),
    );
  });

  it("keeps the add dialog open when the update fails", async () => {
    mockUpdateAgent.mockRejectedValueOnce(new Error("Attachment name already exists"));
    showAttachments({ ...attachment, name: "other" });
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /github GitHub MCP server/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    expect(await within(dialog).findByText("Attachment name already exists")).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("rejects a duplicate custom name without replacing the existing update payload", async () => {
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Custom" }));
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "existing" },
    });
    fireEvent.change(within(dialog).getByLabelText("URL"), {
      target: { value: "https://replacement.example/mcp" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    expect(
      await within(dialog).findByText("An MCP attachment named “existing” already exists."),
    ).toBeInTheDocument();
    expect(mockUpdateAgent).not.toHaveBeenCalled();
  });

  it("disables a preset that would replace an inherited effective attachment", () => {
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const duplicatePreset = within(screen.getByRole("dialog")).getByRole("button", {
      name: /github.*Already attached/i,
    });

    expect(duplicatePreset).toBeDisabled();
    expect(mockUpdateAgent).not.toHaveBeenCalled();
  });
  it("confirms removal without revoking existing grants", async () => {
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("Existing user and service grants are not revoked.");
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove attachment" }));

    await waitFor(() =>
      expect(mockUpdateAgent).toHaveBeenCalledWith({
        agentId: "agent-1",
        request: { mcpServers: agent.mcpServers },
      }),
    );
    expect(mockRevokeConnection).not.toHaveBeenCalled();
  });
});
