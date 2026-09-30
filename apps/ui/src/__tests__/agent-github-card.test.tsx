import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentGitHubCard } from "@/components/agents/agent-github-card";
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
  account: null,
  repository_selection: null,
};

function renderCard() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AgentGitHubCard agentId="agent_1" />
    </QueryClientProvider>,
  );
}

describe("AgentGitHubCard", () => {
  beforeEach(() => jest.resetAllMocks());

  it("submits a manifest form for create_app", async () => {
    mocked.getAgentGitHub.mockResolvedValue(base);
    mocked.connectAgentGitHub.mockResolvedValue({
      kind: "create_app",
      action: "https://github.com/settings/apps/new",
      manifest: '{"name":"x"}',
    });
    const submit = jest.spyOn(HTMLFormElement.prototype, "submit").mockImplementation(() => {});
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
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
    const buttons = await screen.findAllByRole("button", { name: "Disconnect" });
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() => expect(mocked.disconnectAgentGitHub).toHaveBeenCalledWith("ident_1"));
  });
});
