import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AgentCredentialsPanel } from "@/components/agents/agent-credentials-panel";

const setValue = jest.fn();
const remove = jest.fn();

jest.mock("@/hooks/use-agents", () => ({
  useAgentCredentials: () => ({
    data: [
      {
        id: "binding-1",
        agent_id: "agent-1",
        mcp_server_name: "visti",
        mcp_server_url: "https://example.com/mcp",
        tool_name: "visti_send",
        parameter_name: "channel_key",
        label: "Visti channel key",
        configured: false,
        created_at: "2026-08-08T00:00:00Z",
        updated_at: "2026-08-08T00:00:00Z",
        setup_url: "/agents/agent-1?tab=credentials",
      },
    ],
    isLoading: false,
    error: null,
  }),
  useCreateAgentCredentialBinding: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useDeleteAgentCredentialBinding: () => ({ mutateAsync: remove, isPending: false }),
  useSetAgentCredentialValue: () => ({ mutateAsync: setValue, isPending: false }),
}));

describe("AgentCredentialsPanel", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    setValue.mockResolvedValue({ configured: true });
  });

  it("renders metadata without a credential value or prefill", () => {
    render(<AgentCredentialsPanel agentId="agent-1" />);

    expect(screen.getByText("Visti channel key")).toBeInTheDocument();
    expect(screen.getByText("Setup required")).toBeInTheDocument();
    const input = screen.getByLabelText("Value") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.autocomplete).toBe("new-password");
    expect(input.value).toBe("");
  });

  it("clears and removes plaintext client state after submission", async () => {
    const sentinel = "ui-sentinel-must-not-remain";
    render(<AgentCredentialsPanel agentId="agent-1" />);
    const input = screen.getByLabelText("Value");

    fireEvent.change(input, { target: { value: sentinel } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(setValue).toHaveBeenCalledWith({ bindingId: "binding-1", value: sentinel }),
    );
    await waitFor(() => expect(screen.queryByDisplayValue(sentinel)).not.toBeInTheDocument());
    expect(document.body.textContent).not.toContain(sentinel);
  });
});
