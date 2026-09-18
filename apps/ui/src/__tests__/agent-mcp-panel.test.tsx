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
  overridden_sources: [
    { source: "harness", source_label: "Harness: researcher" },
    { source: "capability", source_label: "Capability: web search" },
  ],
  acts_as: "user",
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

  it("keeps capability attachments read-only and links to the capability", () => {
    showAttachments({
      ...attachment,
      source: "capability",
      source_label: "Capability: web search",
      editable: false,
    });

    render(<AgentMcpPanel agent={agent} />);

    expect(screen.getByRole("link", { name: "View capability" })).toHaveAttribute(
      "href",
      "/capabilities",
    );
    expect(screen.queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();
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
      "/mcp-servers",
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
    render(<AgentMcpPanel agent={agent} />);

    fireEvent.click(screen.getByRole("button", { name: "Add MCP server" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /github GitHub MCP server/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Add server" }));

    expect(await within(dialog).findByText("Attachment name already exists")).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
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
