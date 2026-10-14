import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EverrunsGitHubAppSettings } from "@/components/organizations/everruns-github-app-settings";
import * as api from "@/lib/api/everruns-github-app";

jest.mock("@/lib/api/everruns-github-app", () => ({
  ...jest.requireActual("@/lib/api/everruns-github-app"),
  listEverrunsAppLinks: jest.fn(),
  unlinkEverrunsApp: jest.fn(),
}));
jest.mock("@/lib/api/client", () => ({
  api: {},
  getBackendUrl: () => "https://api.example.test",
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1", name: "Acme", role: "admin" } }),
}));
const mocked = jest.mocked(api);

const linked: api.EverrunsAppLinks = {
  configured: true,
  app_slug: "everruns",
  installations: [
    {
      id: "link_1",
      installation_id: 4242,
      account: "acme",
      account_type: "Organization",
      repository_selection: "selected",
      agent_count: 2,
      linked_at: "2026-10-12T00:00:00Z",
    },
  ],
};

function renderSettings() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <EverrunsGitHubAppSettings />
    </QueryClientProvider>,
  );
}

describe("EverrunsGitHubAppSettings", () => {
  beforeEach(() => {
    mocked.listEverrunsAppLinks.mockReset();
    mocked.unlinkEverrunsApp.mockReset();
  });

  it("lists linked accounts and links another through GitHub", async () => {
    mocked.listEverrunsAppLinks.mockResolvedValue(linked);
    renderSettings();

    expect(await screen.findByRole("heading", { name: "Everruns GitHub App" })).toBeInTheDocument();
    expect(screen.getByText("acme")).toBeInTheDocument();
    expect(screen.getByText("Organization · Selected repositories · 2 agents")).toBeInTheDocument();
    expect(screen.getByText(/everruns\[bot\]/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Link/ })).toHaveAttribute(
      "href",
      "https://api.example.test/v1/orgs/org_1/github/everruns-app/authorize?return_to=%2Fsettings%2Forganization",
    );
    expect(mocked.listEverrunsAppLinks).toHaveBeenCalledWith("org_1");
  });

  it("unlinks after confirming that agents lose the account", async () => {
    mocked.listEverrunsAppLinks.mockResolvedValue(linked);
    mocked.unlinkEverrunsApp.mockResolvedValue();
    renderSettings();

    fireEvent.click(await screen.findByRole("button", { name: "Unlink" }));
    expect(await screen.findByText(/lose their GitHub identity/)).toBeInTheDocument();
    const buttons = screen.getAllByRole("button", { name: "Unlink" });
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() => expect(mocked.unlinkEverrunsApp).toHaveBeenCalledWith("org_1", "link_1"));
  });

  it("shows an empty state before anything is linked", async () => {
    mocked.listEverrunsAppLinks.mockResolvedValue({ ...linked, installations: [] });
    renderSettings();
    expect(await screen.findByText("No GitHub account is linked yet.")).toBeInTheDocument();
  });

  it("renders nothing when the deployment has no shared App", async () => {
    mocked.listEverrunsAppLinks.mockResolvedValue({
      configured: false,
      app_slug: null,
      installations: [],
    });
    const { container } = renderSettings();
    await waitFor(() => expect(mocked.listEverrunsAppLinks).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
