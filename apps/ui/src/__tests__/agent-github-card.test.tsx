import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentGitHubCard, AgentGitHubSection } from "@/components/agents/agent-github-card";
import { navigateTo } from "@/lib/browser-navigation";
import * as api from "@/lib/api/agent-github";

jest.mock("@/lib/api/agent-github");
jest.mock("@/lib/browser-navigation", () => ({ navigateTo: jest.fn() }));
const mocked = jest.mocked(api);

const base: api.AgentGitHubStatus = {
  identity_id: null,
  connected: false,
  app_created: false,
  app_slug: null,
  app_url: null,
  kind: null,
  bot_login: null,
  app_settings_url: null,
  account: null,
  repository_selection: null,
  missing_permissions: [],
  suggested_owners: [],
  everruns_app_installations: [],
};

const CREATE_OWN = "Create its App";
const USE_SHARED = "Use the shared App";

/** Opens the create-own-App menu and picks `item`. */
async function connectVia(item: string) {
  fireEvent.click(await screen.findByRole("button", { name: CREATE_OWN }));
  fireEvent.click(await screen.findByRole("menuitem", { name: item }));
}

function renderCard() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <AgentGitHubCard agentId="agent_1" />
    </QueryClientProvider>,
  );
}

describe("AgentGitHubCard", () => {
  beforeEach(() => jest.resetAllMocks());

  it("submits a manifest form for the personal account", async () => {
    mocked.getAgentGitHub.mockResolvedValue(base);
    mocked.connectAgentGitHub.mockResolvedValue({
      kind: "create_app",
      action: "https://github.com/settings/apps/new",
      manifest: '{"name":"x"}',
    });
    const submit = jest.spyOn(HTMLFormElement.prototype, "submit").mockImplementation(() => {});
    renderCard();
    expect(screen.queryByLabelText("GitHub organization")).not.toBeInTheDocument();
    await connectVia("Personal account");
    await waitFor(() => expect(submit).toHaveBeenCalled());
    expect(mocked.connectAgentGitHub).toHaveBeenCalledWith("agent_1", {
      return_to: "/agents/agent_1?tab=integrations",
    });
    const form = document.querySelector("form") as HTMLFormElement;
    expect(form.method).toBe("post");
    expect(form.action).toBe("https://github.com/settings/apps/new");
    expect((form.elements.namedItem("manifest") as HTMLInputElement).value).toBe('{"name":"x"}');
    submit.mockRestore();
  });

  it("offers organizations the org already knows, without personal accounts", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...base,
      suggested_owners: [
        { login: "acme", kind: "organization" },
        { login: "octocat", kind: "user" },
        { login: "beta-labs", kind: "unknown" },
      ],
    });
    mocked.connectAgentGitHub.mockResolvedValue({
      kind: "create_app",
      action: "https://github.com/organizations/beta-labs/settings/apps/new",
      manifest: '{"name":"x"}',
    });
    const submit = jest.spyOn(HTMLFormElement.prototype, "submit").mockImplementation(() => {});
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: CREATE_OWN }));
    expect(
      (await screen.findAllByRole("menuitem")).map((item) => item.textContent?.trim()),
    ).toEqual(["Personal account", "acme", "beta-labs", "Another organization…"]);
    fireEvent.click(screen.getByRole("menuitem", { name: "beta-labs" }));
    await waitFor(() => expect(submit).toHaveBeenCalled());
    expect(mocked.connectAgentGitHub).toHaveBeenCalledWith("agent_1", {
      return_to: "/agents/agent_1?tab=integrations",
      owner_org: "beta-labs",
    });
    submit.mockRestore();
  });

  it("creates the App in another organization the user names", async () => {
    mocked.getAgentGitHub.mockResolvedValue(base);
    mocked.connectAgentGitHub.mockResolvedValue({
      kind: "create_app",
      action: "https://github.com/organizations/acme/settings/apps/new",
      manifest: '{"name":"x"}',
    });
    const submit = jest.spyOn(HTMLFormElement.prototype, "submit").mockImplementation(() => {});
    renderCard();
    await connectVia("Another organization…");
    const connectButton = await screen.findByRole("button", { name: "Connect" });
    expect(connectButton).toBeDisabled();
    fireEvent.change(screen.getByLabelText("GitHub organization"), {
      target: { value: " acme " },
    });
    fireEvent.click(connectButton);
    await waitFor(() => expect(submit).toHaveBeenCalled());
    expect(mocked.connectAgentGitHub).toHaveBeenCalledWith("agent_1", {
      return_to: "/agents/agent_1?tab=integrations",
      owner_org: "acme",
    });
    submit.mockRestore();
  });

  it("navigates for install and labels finish installing", async () => {
    mocked.getAgentGitHub.mockResolvedValue({ ...base, app_created: true });
    mocked.connectAgentGitHub.mockResolvedValue({
      kind: "install",
      url: "https://github.com/apps/x/installations/new",
      app_slug: "x",
    });
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Finish installing" }));
    await waitFor(() =>
      expect(navigateTo).toHaveBeenCalledWith("https://github.com/apps/x/installations/new"),
    );
  });

  it("forgets an App that was never installed", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...base,
      app_created: true,
      identity_id: "ident_1",
    });
    mocked.disconnectAgentGitHub.mockResolvedValue();
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "GitHub actions" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Start over" }));
    await waitFor(() => expect(mocked.disconnectAgentGitHub).toHaveBeenCalledWith("ident_1"));
  });

  it("shows account and disconnects after confirmation", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...base,
      connected: true,
      identity_id: "ident_1",
      account: "acme",
      app_url: "https://github.com/apps/x",
      repository_selection: "selected",
    });
    mocked.disconnectAgentGitHub.mockResolvedValue();
    renderCard();
    expect(await screen.findByText("acme")).toBeInTheDocument();
    expect(screen.getByText("Selected repositories")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Add pull request trigger/ })).toHaveAttribute(
      "href",
      "/agents/agent_1/triggers/new?type=github",
    );
    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
    const buttons = await screen.findAllByRole("button", {
      name: "Disconnect",
    });
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() => expect(mocked.disconnectAgentGitHub).toHaveBeenCalledWith("ident_1"));
  });

  it("asks for permissions an older App lacks", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...base,
      connected: true,
      identity_id: "ident_1",
      missing_permissions: ["actions:read", "checks:read"],
    });
    renderCard();
    expect(await screen.findByRole("status")).toHaveTextContent("actions:read, checks:read");
  });

  it("offers only the agent's own App when the org linked no Everruns App account", async () => {
    mocked.getAgentGitHub.mockResolvedValue(base);
    renderCard();
    expect(await screen.findByRole("button", { name: CREATE_OWN })).toBeInTheDocument();
    // A single option is not "recommended" over anything.
    expect(screen.queryByText("Recommended")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: USE_SHARED })).not.toBeInTheDocument();
  });

  it("lets the agent choose an account the org linked to the Everruns App", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...base,
      everruns_app_installations: [
        {
          connection_id: "link_1",
          account: "acme",
          account_type: "Organization",
          repository_selection: "all",
        },
      ],
    });
    mocked.chooseAgentEverrunsApp.mockResolvedValue({
      ...base,
      connected: true,
      identity_id: "ident_1",
      kind: "everruns_app",
      bot_login: "everruns[bot]",
      account: "acme",
      repository_selection: "all",
    });
    renderCard();
    expect(await screen.findByRole("button", { name: CREATE_OWN })).toBeInTheDocument();
    expect(screen.getByText("Recommended")).toBeInTheDocument();
    expect(screen.getByText(/already connected: acme\./)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: USE_SHARED }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "acme" }));
    await waitFor(() =>
      expect(mocked.chooseAgentEverrunsApp).toHaveBeenCalledWith("agent_1", "link_1"),
    );
    expect(await screen.findByRole("heading", { name: "everruns[bot]" })).toBeInTheDocument();
    expect(screen.getByText("Shared Everruns App")).toBeInTheDocument();
    expect(mocked.connectAgentGitHub).not.toHaveBeenCalled();
  });

  it("does not offer the Everruns App once the agent's own App exists", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...base,
      app_created: true,
      everruns_app_installations: [
        {
          connection_id: "link_1",
          account: "acme",
          account_type: null,
          repository_selection: null,
        },
      ],
    });
    renderCard();
    expect(await screen.findByRole("button", { name: "Finish installing" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: USE_SHARED })).not.toBeInTheDocument();
  });

  it("stays quiet when the App has every permission", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...base,
      connected: true,
      account: "acme",
    });
    renderCard();
    expect(await screen.findByText("acme")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
});

const connectedApp: api.AgentGitHubStatus = {
  ...base,
  connected: true,
  app_created: true,
  identity_id: "ident_1",
  app_slug: "pr-summarizer",
  app_url: "https://github.com/apps/pr-summarizer",
  kind: "agent_app",
  bot_login: "pr-summarizer[bot]",
  app_settings_url: "https://github.com/organizations/acme/settings/apps/pr-summarizer",
  account: "acme",
  repository_selection: "all",
};

function renderSection(props: Partial<Parameters<typeof AgentGitHubSection>[0]> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AgentGitHubSection agentId="agent_1" {...props} />
    </QueryClientProvider>,
  );
}

describe("AgentGitHubSection", () => {
  beforeEach(() => jest.resetAllMocks());

  it("is its own GitHub section showing the bot identity and installation", async () => {
    mocked.getAgentGitHub.mockResolvedValue(connectedApp);
    renderSection({ githubTriggerCount: 2 });

    expect(screen.getByRole("heading", { level: 3, name: "GitHub" })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "pr-summarizer[bot]" })).toBeInTheDocument();
    expect(screen.getByText("Its own GitHub App")).toBeInTheDocument();
    expect(screen.getByText("acme")).toBeInTheDocument();
    expect(screen.getByText("All repositories")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Add pull request trigger/ })).toHaveAttribute(
      "href",
      "/agents/agent_1/triggers/new?type=github",
    );
    expect(screen.getByRole("link", { name: "2 GitHub triggers" })).toHaveAttribute(
      "href",
      "/agents/agent_1?tab=integrations#triggers",
    );
  });

  it("guides setting the avatar with a download and the App settings link", async () => {
    mocked.getAgentGitHub.mockResolvedValue(connectedApp);
    renderSection({
      avatar: {
        id: "av_1",
        url: "/v1/avatars/av_1/square-256.png",
        circle_url: "",
        sizes: [64, 512],
      },
    });

    expect(await screen.findByRole("heading", { name: "Set the avatar" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Download avatar/ })).toHaveAttribute(
      "href",
      expect.stringMatching(/\/v1\/avatars\/av_1\/square-512\.png$/),
    );
    expect(screen.getByRole("link", { name: /Open App settings/ })).toHaveAttribute(
      "href",
      "https://github.com/organizations/acme/settings/apps/pr-summarizer",
    );
  });

  it("links to the App when its settings path is unknown and the agent has no avatar", async () => {
    mocked.getAgentGitHub.mockResolvedValue({ ...connectedApp, app_settings_url: null });
    renderSection();

    expect(await screen.findByRole("heading", { name: "Set the avatar" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /Download avatar/ })).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open App settings/ })).toHaveAttribute(
      "href",
      "https://github.com/apps/pr-summarizer",
    );
    expect(screen.getByRole("link", { name: "GitHub triggers" })).toBeInTheDocument();
  });

  it("labels the shared Everruns App and skips the avatar step", async () => {
    mocked.getAgentGitHub.mockResolvedValue({
      ...connectedApp,
      kind: "everruns_app",
      bot_login: null,
      app_settings_url: null,
    });
    renderSection();

    expect(await screen.findByText("Shared Everruns App")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Set the avatar" })).not.toBeInTheDocument();
  });

  it("offers to connect and shows no avatar step before an installation", async () => {
    mocked.getAgentGitHub.mockResolvedValue(base);
    renderSection();

    expect(await screen.findByRole("button", { name: CREATE_OWN })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Set the avatar" })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: /Add pull request trigger/ }),
    ).not.toBeInTheDocument();
  });
});
