import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AgentCredentialsPanel } from "@/components/agents/agent-credentials-panel";

const setValue = jest.fn();
const remove = jest.fn();
const create = jest.fn();

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
      {
        id: "binding-2",
        agent_id: "agent-1",
        kind: "http",
        env_name: "JEV_API_KEY",
        binding: { url: "https://api.typesafe.ai", auth: { type: "bearer" } },
        label: "Jev API key",
        configured: true,
        created_at: "2026-10-12T00:00:00Z",
        updated_at: "2026-10-12T00:00:00Z",
        setup_url: "/agents/agent-1?tab=credentials",
      },
    ],
    isLoading: false,
    error: null,
  }),
  useCreateAgentCredentialBinding: () => ({ mutateAsync: create, isPending: false }),
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

  it("shows where an HTTP API credential may be sent", () => {
    render(<AgentCredentialsPanel agentId="agent-1" />);

    expect(screen.getByText("Jev API key")).toBeInTheDocument();
    expect(
      screen.getByText("$JEV_API_KEY → https://api.typesafe.ai (Authorization: Bearer)"),
    ).toBeInTheDocument();
  });

  it("declares an HTTP API credential, then stores its value", async () => {
    create.mockResolvedValue({ id: "binding-3" });
    render(<AgentCredentialsPanel agentId="agent-1" />);

    fireEvent.click(screen.getByRole("button", { name: /Add binding/ }));
    fireEvent.change(screen.getByLabelText("Shell variable"), {
      target: { value: "JEV_API_KEY" },
    });
    fireEvent.change(screen.getByLabelText("Sent only to (HTTPS URL prefix)"), {
      target: { value: "https://api.typesafe.ai" },
    });
    fireEvent.change(screen.getByLabelText("Label"), { target: { value: "Jev API key" } });
    fireEvent.change(screen.getByLabelText("Value", { selector: "#credential-new-value" }), {
      target: { value: "jev-secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save binding" }));

    await waitFor(() =>
      expect(create).toHaveBeenCalledWith({
        env_name: "JEV_API_KEY",
        url: "https://api.typesafe.ai",
        auth: "bearer",
        header_name: undefined,
        label: "Jev API key",
        description: undefined,
      }),
    );
    await waitFor(() =>
      expect(setValue).toHaveBeenCalledWith({ bindingId: "binding-3", value: "jev-secret" }),
    );
  });
});
