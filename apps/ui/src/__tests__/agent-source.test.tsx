import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentSourcePanel } from "@/components/agents/agent-source";
import * as api from "@/lib/api/agent-source";
import * as github from "@/lib/api/agent-github";

jest.mock("@/lib/api/agent-source");
jest.mock("@/lib/api/agent-github");
let mockCanManage = true;
jest.mock("@/hooks/use-policies", () => ({ usePolicies: () => ({ can: () => mockCanManage }) }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1" }, isLoading: false }),
}));
jest.mock("@/components/agents/agent-package-review", () => ({
  AgentPackageReview: () => <div>Package change preview</div>,
}));
const mocked = jest.mocked(api);
const gh = jest.mocked(github);
const source: api.AgentSource = {
  mode: "pull",
  local_dirty: false,
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
function Source({ disabled = false }: { disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)}>Sync from GitHub</button>
      <AgentSourcePanel agentId="agent_1" disabled={disabled} open={open} onOpenChange={setOpen} />
    </>
  );
}
function show(disabled = false) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Source disabled={disabled} />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  jest.resetAllMocks();
  mockCanManage = true;
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
      mode: "pull",
    }),
  );
});
it("requires explicit two-way selection and review before enabling repository writes", async () => {
  mocked.previewAgentSource.mockResolvedValue({
    valid: true,
    commit: "a".repeat(40),
    agent_digest: "base",
    changes: [],
  });
  mocked.connectAgentSource.mockResolvedValue({ ...source, mode: "two_way" });
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Sync from GitHub" }));
  expect(screen.getByLabelText("Sync direction")).toHaveValue("pull");
  fireEvent.change(screen.getByLabelText("Sync direction"), { target: { value: "two_way" } });
  expect(screen.getByText(/Requires Contents write permission/)).toBeInTheDocument();
  await screen.findByRole("option", { name: "acme/agents" });
  fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "acme/agents" } });
  await waitFor(() => expect(screen.getByLabelText("Branch")).toHaveValue("main"));
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Enable continuous sync" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Enable continuous sync" }));
  await waitFor(() =>
    expect(mocked.connectAgentSource).toHaveBeenCalledWith(
      "agent_1",
      expect.objectContaining({ mode: "two_way", commit: "a".repeat(40), agent_digest: "base" }),
    ),
  );
});

it("shows retained local edits and a review action for a two-way conflict", async () => {
  mocked.getAgentSource.mockResolvedValue({
    ...source,
    mode: "two_way",
    local_dirty: true,
    status: "conflict",
    error: "Both sides changed",
  });
  show();
  expect(await screen.findByText("Conflict")).toBeInTheDocument();
  expect(screen.getByText(/Your local definition is retained/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Review GitHub version" }));
  expect(screen.getByLabelText("Sync direction")).toHaveValue("two_way");
  expect(screen.getByRole("button", { name: "Enable continuous sync" })).toBeDisabled();
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
  expect(screen.getByText(/current definition stays/i)).toBeInTheDocument();
  expect(mocked.disconnectAgentSource).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Disconnect source sync" }));
  await waitFor(() => expect(mocked.disconnectAgentSource).toHaveBeenCalledWith("agent_1"));
});

it("omits the disconnected Source card and resets a cancelled connection form", async () => {
  show();
  await waitFor(() => expect(screen.queryByText("Loading source…")).not.toBeInTheDocument());
  expect(screen.queryByRole("region", { name: "Agent source" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sync from GitHub" }));
  await screen.findByRole("option", { name: "acme/agents" });
  fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "acme/agents" } });
  await waitFor(() => expect(screen.getByLabelText("Branch")).toHaveValue("main"));
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  fireEvent.click(screen.getByRole("button", { name: "Sync from GitHub" }));
  expect(screen.getByLabelText("Repository")).toHaveValue("");
  expect(screen.getByRole("button", { name: "Enable continuous sync" })).toBeDisabled();
});
it.each(["pending", "syncing"])(
  "does not submit another sync while the source is %s",
  async (status) => {
    mocked.getAgentSource.mockResolvedValue({ ...source, status } as api.AgentSource);
    show();
    expect(await screen.findByRole("button", { name: "Sync now" })).toBeDisabled();
    expect(mocked.syncAgentSource).not.toHaveBeenCalled();
  },
);
it("keeps source status visible to a reader without management controls", async () => {
  mockCanManage = false;
  mocked.getAgentSource.mockResolvedValue(source);
  show();
  await screen.findByText("aaaaaaa");
  expect(screen.getByRole("link", { name: /View on GitHub/ })).toHaveAttribute(
    "href",
    expect.stringContaining(source.applied_commit!),
  );
  expect(screen.queryByRole("button", { name: "Sync now" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sync from GitHub" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
it("shows source lookup failures instead of an empty state", async () => {
  mocked.getAgentSource.mockRejectedValue(new Error("Unavailable"));
  show();
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load Agent source");
  fireEvent.click(screen.getByRole("button", { name: "Sync from GitHub" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
it("retains sync request failures and lets the user retry", async () => {
  mocked.getAgentSource.mockResolvedValue(source);
  mocked.syncAgentSource
    .mockRejectedValueOnce(new Error("Temporary sync failure"))
    .mockResolvedValueOnce({ ...source, status: "pending" });
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Sync now" }));
  await screen.findByText("Temporary sync failure");
  expect(screen.getByText("aaaaaaa")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
  await waitFor(() => expect(mocked.syncAgentSource).toHaveBeenCalledTimes(2));
  expect(screen.queryByText("Temporary sync failure")).not.toBeInTheDocument();
});
