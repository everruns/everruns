import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import ChatPage from "@/app/chat/page";

// Mock the useAgents hook
const mockUseAgents = jest.fn();
jest.mock("@/hooks/use-agents", () => ({
  useAgents: () => mockUseAgents(),
}));

// Mock the Header component
jest.mock("@/components/layout/header", () => ({
  Header: ({ title }: { title: string }) => <header data-testid="header">{title}</header>,
}));

// Mock the AgentSelector component
jest.mock("@/components/chat/agent-selector", () => ({
  AgentSelector: ({
    agents,
    selectedAgentId,
    onAgentChange,
    disabled,
  }: {
    agents: Array<{ id: string; name: string }>;
    selectedAgentId: string | null;
    onAgentChange: (id: string) => void;
    disabled: boolean;
  }) => (
    <div data-testid="agent-selector">
      <select
        data-testid="agent-select"
        value={selectedAgentId || ""}
        onChange={(e) => onAgentChange(e.target.value)}
        disabled={disabled}
      >
        <option value="">Select agent</option>
        {agents.map((agent) => (
          <option key={agent.id} value={agent.id}>
            {agent.name}
          </option>
        ))}
      </select>
    </div>
  ),
}));

// Mock UI components
jest.mock("@/components/ui/card", () => ({
  Card: ({ children, className }: { children: React.ReactNode; className?: string }) => (
    <div data-testid="card" className={className}>{children}</div>
  ),
  CardContent: ({ children, className }: { children: React.ReactNode; className?: string }) => (
    <div data-testid="card-content" className={className}>{children}</div>
  ),
}));

jest.mock("@/components/ui/skeleton", () => ({
  Skeleton: ({ className }: { className?: string }) => (
    <div data-testid="skeleton" className={className} />
  ),
}));

jest.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    onClick,
    disabled,
    ...props
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
  }) => (
    <button onClick={onClick} disabled={disabled} data-testid="button" {...props}>
      {children}
    </button>
  ),
}));

jest.mock("@/components/ui/textarea", () => ({
  Textarea: ({
    value,
    onChange,
    placeholder,
    disabled,
    onKeyDown,
    className,
  }: {
    value: string;
    onChange: (e: React.ChangeEvent<HTMLTextAreaElement>) => void;
    placeholder?: string;
    disabled?: boolean;
    onKeyDown?: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void;
    className?: string;
  }) => (
    <textarea
      data-testid="message-input"
      value={value}
      onChange={onChange}
      placeholder={placeholder}
      disabled={disabled}
      onKeyDown={onKeyDown}
      className={className}
    />
  ),
}));

jest.mock("@/components/ui/scroll-area", () => ({
  ScrollArea: ({ children, className }: { children: React.ReactNode; className?: string }) => (
    <div data-testid="scroll-area" className={className}>{children}</div>
  ),
}));

describe("ChatPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe("Header", () => {
    it("renders with title 'Chat'", () => {
      mockUseAgents.mockReturnValue({
        data: [],
        isLoading: false,
      });

      render(<ChatPage />);

      const header = screen.getByTestId("header");
      expect(header).toHaveTextContent("Chat");
    });

    it("does not render with AG-UI in title", () => {
      mockUseAgents.mockReturnValue({
        data: [],
        isLoading: false,
      });

      render(<ChatPage />);

      const header = screen.getByTestId("header");
      expect(header).not.toHaveTextContent("AG-UI");
      expect(header).not.toHaveTextContent("CopilotKit");
    });
  });

  describe("Loading state", () => {
    it("shows skeleton when agents are loading", () => {
      mockUseAgents.mockReturnValue({
        data: [],
        isLoading: true,
      });

      render(<ChatPage />);

      expect(screen.getByTestId("skeleton")).toBeInTheDocument();
    });
  });

  describe("No agents state", () => {
    it("shows 'No agents available' message when no agents exist", () => {
      mockUseAgents.mockReturnValue({
        data: [],
        isLoading: false,
      });

      render(<ChatPage />);

      expect(screen.getByText("No agents available")).toBeInTheDocument();
      expect(screen.getByText("Create an agent to start chatting.")).toBeInTheDocument();
    });

    it("shows Create Agent button linking to /agents/new", () => {
      mockUseAgents.mockReturnValue({
        data: [],
        isLoading: false,
      });

      render(<ChatPage />);

      const createButton = screen.getByRole("button", { name: /create agent/i });
      expect(createButton).toBeInTheDocument();
    });
  });

  describe("Agent selection state", () => {
    const mockAgents = [
      { id: "agent-1", name: "Test Agent 1", status: "active" },
      { id: "agent-2", name: "Test Agent 2", status: "active" },
      { id: "agent-3", name: "Inactive Agent", status: "inactive" },
    ];

    it("shows agent selector when agents exist", () => {
      mockUseAgents.mockReturnValue({
        data: mockAgents,
        isLoading: false,
      });

      render(<ChatPage />);

      expect(screen.getByTestId("agent-selector")).toBeInTheDocument();
    });

    it("filters to only active agents in selector", () => {
      mockUseAgents.mockReturnValue({
        data: mockAgents,
        isLoading: false,
      });

      render(<ChatPage />);

      const select = screen.getByTestId("agent-select");
      expect(select).toBeInTheDocument();

      // Check that active agents are available
      expect(screen.getByText("Test Agent 1")).toBeInTheDocument();
      expect(screen.getByText("Test Agent 2")).toBeInTheDocument();
      // Inactive agent should not be in options
      expect(screen.queryByText("Inactive Agent")).not.toBeInTheDocument();
    });

    it("shows 'Select an agent to start' when no agent is selected", () => {
      mockUseAgents.mockReturnValue({
        data: mockAgents,
        isLoading: false,
      });

      render(<ChatPage />);

      expect(screen.getByText("Select an agent to start")).toBeInTheDocument();
      expect(screen.getByText("Choose an agent from the dropdown above")).toBeInTheDocument();
    });

    it("shows chat interface when agent is selected", async () => {
      mockUseAgents.mockReturnValue({
        data: mockAgents,
        isLoading: false,
      });

      render(<ChatPage />);

      // Select an agent
      const select = screen.getByTestId("agent-select");
      fireEvent.change(select, { target: { value: "agent-1" } });

      await waitFor(() => {
        expect(screen.getByTestId("message-input")).toBeInTheDocument();
      });
    });
  });

  describe("Chat interface", () => {
    const mockAgents = [
      { id: "agent-1", name: "Test Agent", status: "active" },
    ];

    beforeEach(() => {
      mockUseAgents.mockReturnValue({
        data: mockAgents,
        isLoading: false,
      });
    });

    it("renders message input after selecting agent", async () => {
      render(<ChatPage />);

      const select = screen.getByTestId("agent-select");
      fireEvent.change(select, { target: { value: "agent-1" } });

      await waitFor(() => {
        expect(screen.getByTestId("message-input")).toBeInTheDocument();
      });
    });

    it("renders send button after selecting agent", async () => {
      render(<ChatPage />);

      const select = screen.getByTestId("agent-select");
      fireEvent.change(select, { target: { value: "agent-1" } });

      await waitFor(() => {
        const buttons = screen.getAllByTestId("button");
        const sendButton = buttons.find(btn => btn.querySelector("svg"));
        expect(sendButton).toBeInTheDocument();
      });
    });

    it("has placeholder with agent name in input", async () => {
      render(<ChatPage />);

      const select = screen.getByTestId("agent-select");
      fireEvent.change(select, { target: { value: "agent-1" } });

      await waitFor(() => {
        const input = screen.getByTestId("message-input");
        expect(input).toHaveAttribute("placeholder", "Message Test Agent...");
      });
    });

    it("disables send button when input is empty", async () => {
      render(<ChatPage />);

      const select = screen.getByTestId("agent-select");
      fireEvent.change(select, { target: { value: "agent-1" } });

      await waitFor(() => {
        const buttons = screen.getAllByTestId("button");
        const sendButton = buttons[buttons.length - 1]; // Last button is send
        expect(sendButton).toBeDisabled();
      });
    });

    it("enables send button when input has text", async () => {
      render(<ChatPage />);

      const select = screen.getByTestId("agent-select");
      fireEvent.change(select, { target: { value: "agent-1" } });

      await waitFor(() => {
        const input = screen.getByTestId("message-input");
        fireEvent.change(input, { target: { value: "Hello" } });
      });

      await waitFor(() => {
        const buttons = screen.getAllByTestId("button");
        const sendButton = buttons[buttons.length - 1];
        expect(sendButton).not.toBeDisabled();
      });
    });

    it("clears messages when changing agents", async () => {
      const multiAgents = [
        { id: "agent-1", name: "Agent 1", status: "active" },
        { id: "agent-2", name: "Agent 2", status: "active" },
      ];

      mockUseAgents.mockReturnValue({
        data: multiAgents,
        isLoading: false,
      });

      render(<ChatPage />);

      // Select first agent
      const select = screen.getByTestId("agent-select");
      fireEvent.change(select, { target: { value: "agent-1" } });

      await waitFor(() => {
        expect(screen.getByTestId("message-input")).toBeInTheDocument();
      });

      // Change to second agent
      fireEvent.change(select, { target: { value: "agent-2" } });

      await waitFor(() => {
        // Input should still be there with new agent's placeholder
        const input = screen.getByTestId("message-input");
        expect(input).toHaveAttribute("placeholder", "Message Agent 2...");
      });
    });
  });
});
