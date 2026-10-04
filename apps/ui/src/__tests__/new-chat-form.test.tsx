import * as mockReact from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { NewPlaygroundChatForm } from "@/components/chat/new-chat-form";
import { useAgents, useHarnesses } from "@/hooks";
import { useCreateSession } from "@/hooks/use-sessions";
import type { EnvironmentSet } from "@/lib/api/types";

const mockPush = jest.fn();
jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush }),
}));

jest.mock("@/hooks", () => ({
  useAgents: jest.fn(),
  useHarnesses: jest.fn(),
}));

// ChatMessageList and friends reach `use-providers` for org context this suite
// does not stub. The form itself no longer reads intelligence state — its hosts
// do — so a fixed healthy model is enough.
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({
    data: [{ enabled: true, healthy: true, capabilities: ["chat"] }],
    isLoading: false,
    isError: false,
  }),
  useProvidersConfig: () => ({ data: { policies: {} }, isLoading: false }),
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

  function selectLabel(node: mockReact.ReactNode): string {
    let label = "Select";
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child)) return;
      if ((child.type as { isSelectTrigger?: boolean }).isSelectTrigger) {
        label = (child.props as { "aria-label"?: string })["aria-label"] ?? label;
        return;
      }
      const nested = selectLabel((child.props as { children?: mockReact.ReactNode }).children);
      if (nested !== "Select") label = nested;
    });
    return label;
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
        aria-label={selectLabel(children)}
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      >
        <option value="">Pick an agent or harness</option>
        {collectOptions(children)}
      </select>
    );
  }

  function SelectItem(props: { value: string; children: mockReact.ReactNode }) {
    void props;
    return null;
  }
  SelectItem.isSelectItem = true;
  function SelectTrigger({ children }: { children: mockReact.ReactNode }) {
    return <>{children}</>;
  }
  SelectTrigger.isSelectTrigger = true;

  return {
    Select,
    SelectContent: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectGroup: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectItem,
    SelectLabel: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectTrigger,
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
}: {
  agents?: Array<{
    id: string;
    name: string;
    display_name: string;
    environments?: EnvironmentSet;
  }>;
  harnesses?: Array<{ id: string; name: string; display_name: string }>;
} = {}) {
  mockUseAgents.mockReturnValue({ data: agents, isLoading: false });
  mockUseHarnesses.mockReturnValue({ data: harnesses, isLoading: false });
  mockUseCreateSession.mockReturnValue({ mutateAsync, isPending: false });
}

describe("NewPlaygroundChatForm", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mutateAsync.mockResolvedValue({ id: "sess_new" });
    setup();
  });

  it("binds the chosen end user and opens Playground", async () => {
    render(<NewPlaygroundChatForm endUserId="identity_customer" />);

    fireEvent.change(screen.getByRole("combobox", { name: "Chat counterpart" }), {
      target: { value: "agent:agent_1" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Start Playground chat/ }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        request: {
          agent_id: "agent_1",
          source: "playground",
          playground_user_id: "identity_customer",
        },
      }),
    );
    await waitFor(() => expect(mockPush).toHaveBeenCalledWith("/playground/sess_new"));
  });

  it("shows an agent's environments and pins the selected profile", async () => {
    setup({
      agents: [
        {
          id: "agent_1",
          name: "scout",
          display_name: "Scout",
          environments: {
            default: "scratch",
            profiles: {
              scratch: { target: { kind: "vfs", provider: "bashkit" } },
              build: { target: { kind: "managed", provider: "daytona" } },
            },
          },
        },
      ],
    });
    render(<NewPlaygroundChatForm endUserId="identity_customer" />);

    fireEvent.change(screen.getByRole("combobox", { name: "Chat counterpart" }), {
      target: { value: "agent:agent_1" },
    });
    expect(screen.getByRole("combobox", { name: "Environment" })).toHaveValue("scratch");
    fireEvent.change(screen.getByRole("combobox", { name: "Environment" }), {
      target: { value: "build" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Start Playground chat/ }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        request: {
          agent_id: "agent_1",
          environment: { use: "build" },
          source: "playground",
          playground_user_id: "identity_customer",
        },
      }),
    );
  });

  it("shows a fixed Environment without sending a Session override", async () => {
    setup({
      agents: [
        {
          id: "agent_1",
          name: "scout",
          display_name: "Scout",
          environments: {
            default: "build",
            profiles: {
              build: { target: { kind: "managed", provider: "daytona" } },
            },
          },
        },
      ],
    });

    render(<NewPlaygroundChatForm initialAgentId="agent_1" endUserId="identity_customer" />);

    expect(screen.getByText("build · fixed by Agent")).toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "Environment" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Start Playground chat/ }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        request: {
          agent_id: "agent_1",
          source: "playground",
          playground_user_id: "identity_customer",
        },
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

    render(<NewPlaygroundChatForm endUserId="identity_customer" />);

    expect(screen.getByRole("option", { name: "Generic" })).toHaveValue("harness:generic");
    fireEvent.change(screen.getByRole("combobox", { name: "Chat counterpart" }), {
      target: { value: "harness:generic" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Start Playground chat/ }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        request: {
          harness_name: "generic",
          source: "playground",
          playground_user_id: "identity_customer",
        },
      }),
    );
  });

  it("points at agent creation when there is nothing to talk to", () => {
    setup({ agents: [], harnesses: [] });

    render(<NewPlaygroundChatForm endUserId="identity_customer" />);

    fireEvent.click(screen.getByRole("button", { name: "Create an agent" }));
    expect(mockPush).toHaveBeenCalledWith("/agents/new");
  });
  it("waits for the default virtual user before creation", () => {
    render(<NewPlaygroundChatForm />);
    fireEvent.change(screen.getByRole("combobox", { name: "Chat counterpart" }), {
      target: { value: "agent:agent_1" },
    });
    expect(screen.getByRole("button", { name: /Start Playground chat/ })).toBeDisabled();
  });
});
