import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentSourcePanel } from "@/components/agents/agent-source";
import * as api from "@/lib/api/agent-source";
import * as github from "@/lib/api/agent-github";

jest.mock("@/lib/api/agent-source");
jest.mock("@/lib/api/agent-github");
jest.mock("@/hooks/use-policies", () => ({ usePolicies: () => ({ can: () => true }) }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1" }, isLoading: false }),
}));
jest.mock("@/components/agents/agent-package-review", () => ({
  AgentPackageReview: () => <div>Package change preview</div>,
}));
const mocked = jest.mocked(api);
const gh = jest.mocked(github);
const source: api.AgentSource = {
  generation: "generation-1",
  repository: "acme/agents",
  branch: "main",
  path: "agents/support",
  managed_channels: ["chat"],
  applied_commit: "a".repeat(40),
  status: "synced",
  error: null,
  last_checked_at: "2026-10-19T12:00:00Z",
  last_synced_at: "2026-10-19T12:00:00Z",
};
function show() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <AgentSourcePanel agentId="agent_1" />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  jest.resetAllMocks();
  mocked.getAgentSource.mockResolvedValue(null);
  gh.getAgentGitHub.mockResolvedValue({
    connected: true,
    identity_id: "identity_1",
  } as github.AgentGitHubStatus);
  gh.listGitHubRepositories.mockResolvedValue([
    { full_name: "acme/agents", private: true, html_url: "https://github.com/acme/agents" },
  ]);
  mocked.getAgentSourceOptions.mockResolvedValue({
    default_branch: "main",
    branch: "main",
    branches: ["main"],
    paths: ["agents/"],
  });
});
it("requires a valid preview and sends the exact reviewed commit when enabling sync", async () => {
  mocked.previewAgentSource.mockResolvedValue({
    valid: true,
    preview: { name: "support", instructions: "Help", capabilities: {}, files: {} },
    commit: "a".repeat(40),
    agent_digest: "base",
    changes: [],
  });
  mocked.connectAgentSource.mockResolvedValue(source);
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Sync from GitHub" }));
  expect(screen.getByRole("button", { name: "Enable continuous sync" })).toBeDisabled();
  await screen.findByRole("option", { name: "acme/agents" });
  fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "acme/agents" } });
  await waitFor(() => expect(screen.getByLabelText("Branch")).toHaveValue("main"));
  fireEvent.change(screen.getByLabelText("Package folder or manifest"), {
    target: { value: "agents/support" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  await screen.findByText("Package change preview");
  fireEvent.click(screen.getByRole("button", { name: "Enable continuous sync" }));
  await waitFor(() =>
    expect(mocked.connectAgentSource).toHaveBeenCalledWith("agent_1", {
      repository: "acme/agents",
      branch: "main",
      path: "agents/support",
      commit: "a".repeat(40),
      agent_digest: "base",
    }),
  );
});
it("invalidates a preview when the source selection changes", async () => {
  mocked.previewAgentSource.mockResolvedValue({
    valid: false,
    commit: "a".repeat(40),
    agent_digest: "base",
    diagnostics: [{ path: "dependencies", message: "Missing model" }],
  });
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Sync from GitHub" }));
  await screen.findByRole("option", { name: "acme/agents" });
  fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "acme/agents" } });
  await waitFor(() => expect(screen.getByLabelText("Branch")).toHaveValue("main"));
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  await screen.findByText("dependencies: Missing model");
  expect(screen.getByRole("button", { name: "Enable continuous sync" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Package folder or manifest"), {
    target: { value: "other" },
  });
  expect(screen.queryByText("dependencies: Missing model")).not.toBeInTheDocument();
});
it("shows the retained revision on failure and queues a retry", async () => {
  mocked.getAgentSource.mockResolvedValue({ ...source, status: "error", error: "Missing model" });
  mocked.syncAgentSource.mockResolvedValue({ ...source, status: "pending" });
  show();
  await screen.findByText(/Missing model/);
  expect(screen.getByText("aaaaaaa")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
  await waitFor(() => expect(mocked.syncAgentSource).toHaveBeenCalledWith("agent_1"));
});
it("disconnects only after explaining that the current definition stays", async () => {
  mocked.getAgentSource.mockResolvedValue(source);
  mocked.disconnectAgentSource.mockResolvedValue(undefined);
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
  expect(screen.getByText(/last synced definition stays/i)).toBeInTheDocument();
  expect(mocked.disconnectAgentSource).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Disconnect source sync" }));
  await waitFor(() => expect(mocked.disconnectAgentSource).toHaveBeenCalledWith("agent_1"));
});
