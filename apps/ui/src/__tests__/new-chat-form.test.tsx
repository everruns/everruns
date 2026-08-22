import * as mockReact from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { NewChatForm } from "@/components/chat/new-chat-form";
import { useAgents, useHarnesses } from "@/hooks";
import { useCreateSession } from "@/hooks/use-sessions";

const mockPush = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush }),
}));

jest.mock("@/hooks", () => ({
  useAgents: jest.fn(),
  useHarnesses: jest.fn(),
}));

jest.mock("@/hooks/use-sessions", () => ({
  useCreateSession: jest.fn(),
}));

// Render the design-system Select as a native select so the test can pick an
// option without driving the popup — same shape the other suites use.
jest.mock("@/components/ui/select", () => {
  function collectOptions(node: mockReact.ReactNode): mockReact.ReactNode[] {
    const options: mockReact.ReactNode[] = [];
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child)) return;
      if ((child.type as { isSelectItem?: boolean }).isSelectItem) {
        const props = child.props as { value: string; children: mockReact.ReactNode };
        options.push(
          <option key={props.value} value={props.value}>
            {props.children}
          </option>,
        );
        return;
      }
      options.push(...collectOptions((child.props as { children?: mockReact.ReactNode }).children));
    });
    return options;
  }

  function Select({
    value,
    onValueChange,
    children,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    children: mockReact.ReactNode;
  }) {
    return (
      <select
        aria-label="Chat counterpart"
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      >
        <option value="">Pick an agent or harness</option>
        {collectOptions(children)}
      </select>
    );
  }

  function SelectItem(_: { value: string; children: mockReact.ReactNode }) {
    return null;
  }
  SelectItem.isSelectItem = true;

  return {
    Select,
    SelectContent: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectGroup: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectItem,
    SelectLabel: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectTrigger: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectValue: () => null,
  };
});

const mockUseAgents = jest.mocked(useAgents) as unknown as jest.Mock;
const mockUseHarnesses = jest.mocked(useHarnesses) as unknown as jest.Mock;
const mockUseCreateSession = jest.mocked(useCreateSession) as unknown as jest.Mock;

const mutateAsync = jest.fn();

function setup({
  agents = [{ id: "agent_1", name: "scout", display_name: "Scout" }],
  harnesses = [{ id: "harness_1", name: "platform-chat", display_name: "Platform Chat" }],
} = {}) {
  mockUseAgents.mockReturnValue({ data: agents, isLoading: false });
  mockUseHarnesses.mockReturnValue({ data: harnesses, isLoading: false });
  mockUseCreateSession.mockReturnValue({ mutateAsync, isPending: false });
}

describe("NewChatForm", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mutateAsync.mockResolvedValue({ id: "sess_new" });
    setup();
  });

  it("creates an agent-bound thread and opens it", async () => {
    render(<NewChatForm />);

    fireEvent.change(screen.getByRole("combobox", { name: "Chat counterpart" }), {
      target: { value: "agent:agent_1" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Start chat/ }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        request: { agent_id: "agent_1", tags: ["chat"] },
      }),
    );
    await waitFor(() => expect(mockPush).toHaveBeenCalledWith("/chats/sess_new"));
  });

  it("binds a Platform Chat thread to the harness instead", async () => {
    render(<NewChatForm />);

    fireEvent.change(screen.getByRole("combobox", { name: "Chat counterpart" }), {
      target: { value: "harness:platform-chat" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Start chat/ }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        request: { harness_name: "platform-chat", tags: ["chat"] },
      }),
    );
  });

  it("creates a harness-bound thread without an agent", async () => {
    setup({
      harnesses: [
        {
          id: "harness_generic",
          name: "generic",
          display_name: "Generic",
        },
      ],
    });

    render(<NewChatForm />);

    expect(screen.getByRole("option", { name: "Generic" })).toHaveValue("harness:generic");
    fireEvent.change(screen.getByRole("combobox", { name: "Chat counterpart" }), {
      target: { value: "harness:generic" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Start chat/ }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        request: { harness_name: "generic", tags: ["chat"] },
      }),
    );
  });

  it("points at agent creation when there is nothing to talk to", () => {
    setup({ agents: [], harnesses: [] });

    render(<NewChatForm />);

    fireEvent.click(screen.getByRole("button", { name: "Create an agent" }));
    expect(mockPush).toHaveBeenCalledWith("/agents/new");
  });
});
