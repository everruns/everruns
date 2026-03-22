import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { IdentityConnections } from "@/components/agent-identity/identity-connections";
import type { ConnectionProvider, UserConnection } from "@/lib/api/types";

const mockUseIdentityConnections = jest.fn();
const mockUseDeleteIdentityConnection = jest.fn();
const mockUseVerifyIdentityConnection = jest.fn();
const mockUseConnectionProviders = jest.fn();

jest.mock("@/hooks/use-identity-connections", () => ({
  useIdentityConnections: (identityId: string) => mockUseIdentityConnections(identityId),
  useDeleteIdentityConnection: (identityId: string) => mockUseDeleteIdentityConnection(identityId),
  useVerifyIdentityConnection: (identityId: string) => mockUseVerifyIdentityConnection(identityId),
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useConnectionProviders: () => mockUseConnectionProviders(),
}));

jest.mock("@/components/connections/provider-icon", () => ({
  ProviderIcon: ({ iconName }: { iconName: string }) => <div data-testid={`icon-${iconName}`} />,
}));

jest.mock("@/components/agent-identity/identity-api-key-dialog", () => ({
  IdentityApiKeyDialog: ({
    open,
    provider,
  }: {
    open: boolean;
    provider: ConnectionProvider | null;
  }) => (open ? <div data-testid="identity-api-key-dialog">{provider?.display_name}</div> : null),
}));

const githubProvider: ConnectionProvider = {
  provider_id: "github",
  display_name: "GitHub",
  description: "GitHub API access",
  icon: "github",
  connection_type: "api_key",
  form_schema: {
    fields: [
      {
        name: "api_key",
        label: "API Key",
        field_type: "password",
        required: true,
        placeholder: "Enter API key",
        help_text: null,
      },
    ],
    instructions_markdown: "Paste a GitHub token",
  },
};

const daytonaProvider: ConnectionProvider = {
  provider_id: "daytona",
  display_name: "Daytona",
  description: "Cloud sandbox access",
  icon: "cloud",
  connection_type: "api_key",
  form_schema: {
    fields: [
      {
        name: "api_key",
        label: "API Key",
        field_type: "password",
        required: true,
        placeholder: "Enter API key",
        help_text: null,
      },
    ],
    instructions_markdown: "Paste a Daytona API key",
  },
};

const githubConnection = {
  provider: "github",
  connection_type: "api_key",
  provider_username: "octocat",
  connected_at: "2026-03-01T00:00:00Z",
  scopes: "repo",
} as UserConnection;

describe("IdentityConnections", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(window, "confirm").mockReturnValue(true);

    mockUseIdentityConnections.mockReturnValue({
      data: [githubConnection],
      isLoading: false,
      error: null,
    });
    mockUseDeleteIdentityConnection.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue(undefined),
      isPending: false,
    });
    mockUseVerifyIdentityConnection.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({ valid: true }),
      isPending: false,
    });
    mockUseConnectionProviders.mockReturnValue({
      data: [githubProvider, daytonaProvider],
    });
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("renders connected and available identity providers", () => {
    render(<IdentityConnections identityId="identity_123" />);

    expect(screen.getByText("GitHub")).toBeInTheDocument();
    expect(screen.getByText("octocat")).toBeInTheDocument();
    expect(screen.getByText("Daytona")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Verify" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });

  it("opens the API key dialog for an available provider", async () => {
    render(<IdentityConnections identityId="identity_123" />);

    fireEvent.click(screen.getByRole("button", { name: "Connect" }));

    await waitFor(() => {
      expect(screen.getByTestId("identity-api-key-dialog")).toHaveTextContent("Daytona");
    });
  });

  it("verifies API-key connections and shows success feedback", async () => {
    const verifyMutateAsync = jest.fn().mockResolvedValue({ valid: true });
    mockUseVerifyIdentityConnection.mockReturnValue({
      mutateAsync: verifyMutateAsync,
      isPending: false,
    });

    render(<IdentityConnections identityId="identity_123" />);

    fireEvent.click(screen.getByRole("button", { name: "Verify" }));

    await waitFor(() => {
      expect(verifyMutateAsync).toHaveBeenCalledWith("github");
      expect(screen.getByText("API key is valid")).toBeInTheDocument();
    });
  });

  it("disconnects a provider after confirmation", async () => {
    const deleteMutateAsync = jest.fn().mockResolvedValue(undefined);
    mockUseDeleteIdentityConnection.mockReturnValue({
      mutateAsync: deleteMutateAsync,
      isPending: false,
    });

    render(<IdentityConnections identityId="identity_123" />);

    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));

    await waitFor(() => {
      expect(window.confirm).toHaveBeenCalledWith("Disconnect GitHub from this identity?");
      expect(deleteMutateAsync).toHaveBeenCalledWith("github");
    });
  });
});
