import { fireEvent, render, screen } from "@testing-library/react";
import { ChatMcpServersButton, ChatMcpServersList } from "@/components/chat/chat-mcp-servers";

const mockUseChatMcpServers = jest.fn();
const mockRemove = jest.fn();

jest.mock("@/hooks/use-chat-mcp-servers", () => ({
  useChatMcpServers: (sessionId: string) => mockUseChatMcpServers(sessionId),
  useRemoveChatMcpServer: () => ({ mutate: mockRemove, isPending: false, error: null }),
}));

const servers = [
  {
    name: "linear",
    url: "https://mcp.linear.app/mcp",
    catalog_name: "linear",
    connection: "not_connected",
  },
  { name: "notes", url: "https://notes.example.com/mcp", connection: "not_needed" },
];

function loaded(data: unknown[]) {
  mockUseChatMcpServers.mockReturnValue({
    data,
    isLoading: false,
    error: null,
    refetch: jest.fn(),
  });
}

describe("ChatMcpServersList", () => {
  beforeEach(() => {
    mockRemove.mockReset();
    mockUseChatMcpServers.mockReset();
  });

  it("lists each chat-only server with its host and sign-in state", () => {
    loaded(servers);
    render(<ChatMcpServersList sessionId="session_1" />);
    expect(mockUseChatMcpServers).toHaveBeenCalledWith("session_1");
    expect(screen.getByText("linear")).toBeInTheDocument();
    expect(screen.getByText("mcp.linear.app")).toBeInTheDocument();
    expect(screen.getByText("Catalog")).toBeInTheDocument();
    expect(screen.getByText("Needs sign-in")).toBeInTheDocument();
    expect(screen.getByText("notes.example.com")).toBeInTheDocument();
  });

  it("removes a server by name", () => {
    loaded(servers);
    render(<ChatMcpServersList sessionId="session_1" />);
    fireEvent.click(screen.getByRole("button", { name: "Remove notes" }));
    expect(mockRemove).toHaveBeenCalledWith("notes");
  });

  it("offers no remove action on a read-only chat", () => {
    loaded(servers);
    render(<ChatMcpServersList sessionId="session_1" readOnly />);
    expect(screen.queryByRole("button", { name: /Remove/ })).not.toBeInTheDocument();
  });

  it("says so when the chat has none", () => {
    loaded([]);
    render(<ChatMcpServersList sessionId="session_1" />);
    expect(screen.getByText("No MCP servers were added to this chat.")).toBeInTheDocument();
  });
});

describe("ChatMcpServersButton", () => {
  beforeEach(() => mockUseChatMcpServers.mockReset());

  it("stays hidden until the chat has a chat-only server", () => {
    loaded([]);
    const { container } = render(<ChatMcpServersButton sessionId="session_1" />);
    expect(container).toBeEmptyDOMElement();
  });

  it("opens the list from the header", () => {
    loaded(servers);
    render(<ChatMcpServersButton sessionId="session_1" />);
    fireEvent.click(screen.getByRole("button", { name: "MCP servers in this chat (2)" }));
    expect(screen.getByText("MCP servers in this chat")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove linear" })).toBeInTheDocument();
  });
});
