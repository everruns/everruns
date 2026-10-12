import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ConnectedClientsPanel } from "@/components/connections/connected-clients-panel";

// Connected AI clients on Settings > My agent experience
// (knowledge/integrations/mcp-connected-clients.md, phase 1).
const mockUseConnectedClients = jest.fn();
const mockUseRevokeConnectedClient = jest.fn();

jest.mock("@/hooks/use-connected-clients", () => ({
  useConnectedClients: () => mockUseConnectedClients(),
  useRevokeConnectedClient: () => mockUseRevokeConnectedClient(),
}));

const cursor = {
  id: "01999999-0000-7000-8000-000000000001",
  client_name: "Cursor",
  redirect_hosts: ["anysphere.cursor-retrieval"],
  access: "read_and_run",
  all_organizations: true,
  created_at: "2026-01-02T00:00:00Z",
  last_used_at: null,
};

describe("ConnectedClientsPanel", () => {
  beforeEach(() => {
    mockUseRevokeConnectedClient.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue(undefined),
      reset: jest.fn(),
      isPending: false,
      error: null,
    });
  });

  it("renders the empty state", () => {
    mockUseConnectedClients.mockReturnValue({ data: [], isLoading: false, error: null });

    render(<ConnectedClientsPanel />);

    expect(screen.getByText("No connected AI clients")).toBeInTheDocument();
  });

  it("shows each client with its host, access and organizations", () => {
    mockUseConnectedClients.mockReturnValue({ data: [cursor], isLoading: false, error: null });

    render(<ConnectedClientsPanel />);

    expect(screen.getByText("Cursor")).toBeInTheDocument();
    expect(screen.getByText("C")).toBeInTheDocument();
    expect(screen.getByText("anysphere.cursor-retrieval")).toBeInTheDocument();
    expect(screen.getByText("Read and run")).toBeInTheDocument();
    expect(screen.getByText("All organizations")).toBeInTheDocument();
    expect(screen.getByText("Never")).toBeInTheDocument();
  });

  it("revokes only after confirming", async () => {
    const mutateAsync = jest.fn().mockResolvedValue(undefined);
    mockUseRevokeConnectedClient.mockReturnValue({
      mutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: null,
    });
    mockUseConnectedClients.mockReturnValue({ data: [cursor], isLoading: false, error: null });

    render(<ConnectedClientsPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));

    expect(await screen.findByText("Disconnect Cursor?")).toBeInTheDocument();
    expect(mutateAsync).not.toHaveBeenCalled();

    const buttons = screen.getAllByRole("button", { name: "Revoke" });
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() => expect(mutateAsync).toHaveBeenCalledWith(cursor.id));
  });

  it("cancelling the dialog keeps the client", async () => {
    const mutateAsync = jest.fn();
    mockUseRevokeConnectedClient.mockReturnValue({
      mutateAsync,
      reset: jest.fn(),
      isPending: false,
      error: null,
    });
    mockUseConnectedClients.mockReturnValue({ data: [cursor], isLoading: false, error: null });

    render(<ConnectedClientsPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(screen.queryByText("Disconnect Cursor?")).not.toBeInTheDocument());
    expect(mutateAsync).not.toHaveBeenCalled();
  });
});
