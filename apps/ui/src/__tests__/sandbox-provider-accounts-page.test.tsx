import { render, screen } from "@testing-library/react";
import SandboxProviderAccountsPage from "@/app/(main)/sandbox-provider-accounts/page";

const mutation = { mutateAsync: jest.fn(), isPending: false };

jest.mock("@/hooks/use-organization-connections", () => ({
  useOrganizationConnections: () => ({
    data: [
      {
        id: "connection-1",
        name: "Production Daytona",
        provider: "daytona",
        connected_at: "2026-10-07T00:00:00Z",
        updated_at: "2026-10-07T00:00:00Z",
      },
    ],
    isLoading: false,
    error: null,
  }),
  useSaveOrganizationConnection: () => mutation,
  useDeleteOrganizationConnection: () => mutation,
  useVerifyOrganizationConnection: () => mutation,
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useConnectionProviders: () => ({
    data: [
      {
        provider_id: "daytona",
        display_name: "Daytona",
        description: "Cloud sandboxes",
        icon: "daytona",
        connection_type: "api_key",
        capabilities: ["sandbox_provisioning"],
        form_schema: { fields: [], instructions_markdown: "" },
      },
      {
        provider_id: "e2b",
        display_name: "E2B",
        description: "E2B sandboxes",
        icon: "cloud",
        connection_type: "api_key",
        capabilities: ["sandbox_provisioning"],
        form_schema: { fields: [], instructions_markdown: "" },
      },
      {
        provider_id: "github",
        display_name: "GitHub",
        description: "Repositories",
        icon: "github",
        connection_type: "oauth",
        capabilities: [],
      },
    ],
  }),
}));

describe("Sandbox Provider Accounts", () => {
  it("shows sandbox providers and hides unrelated connectors", () => {
    render(<SandboxProviderAccountsPage />);

    expect(screen.getByText("Production Daytona")).toBeInTheDocument();
    expect(screen.getAllByText("Daytona").length).toBeGreaterThan(0);
    expect(screen.getByText("E2B")).toBeInTheDocument();
    expect(screen.queryByText("GitHub")).not.toBeInTheDocument();
  });
});
