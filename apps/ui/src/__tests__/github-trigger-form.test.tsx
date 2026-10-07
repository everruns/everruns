import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import GitHubTriggerPage from "@/app/(main)/agents/[agentId]/triggers/[triggerId]/page";

const create = jest.fn().mockResolvedValue({});
const push = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push }),
  usePathname: () => "/agents/agent_1/triggers/new",
  useSearchParams: () => new URLSearchParams("type=github"),
}));
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => false,
}));
jest.mock("@/hooks/use-agents", () => ({
  useAgent: () => ({ data: { id: "agent_1", name: "Reviewer" }, isLoading: false }),
}));
jest.mock("@/hooks/use-agent-triggers", () => ({
  useAgentTriggers: () => ({ data: [], isLoading: false }),
  useCreateAgentTrigger: () => ({ mutateAsync: create, isPending: false }),
  useUpdateAgentTrigger: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useDeleteAgentTrigger: () => ({ mutate: jest.fn(), isPending: false }),
  useRunAgentTrigger: () => ({ mutate: jest.fn(), isPending: false }),
}));
let connected = true;
jest.mock("@/hooks/use-agent-github", () => ({
  useAgentGitHub: () => ({
    data: { connected, app_created: connected, identity_id: "ident_1" },
    isLoading: false,
  }),
  useConnectAgentGitHub: () => ({ mutate: jest.fn(), isPending: false }),
  useGitHubRepositories: () => ({
    data: [{ full_name: "acme/api", private: false, html_url: "u" }],
  }),
}));

// A pre-fulfilled thenable lets React's `use()` read params without suspending.
const params = Object.assign(Promise.resolve({ agentId: "agent_1", triggerId: "new" }), {
  status: "fulfilled",
  value: { agentId: "agent_1", triggerId: "new" },
});
const page = () => <GitHubTriggerPage params={params} />;

describe("GitHub trigger page", () => {
  beforeEach(() => {
    connected = true;
    create.mockClear();
  });

  it("creates a github trigger with selected events and repositories", async () => {
    render(page());
    fireEvent.click(await screen.findByRole("checkbox", { name: "Pull request closed" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "acme/api" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(create).toHaveBeenCalled());
    const request = create.mock.calls[0][0];
    expect(request.trigger_type).toBe("github");
    expect(request.session_mode).toBe("per_thread");
    expect(request.repositories).toEqual(["acme/api"]);
    expect(request.github_events).toContain("pull_request.closed");
    expect(request.github_events).toContain("pull_request.opened");
    expect(request.message).toContain("{{github.repository}}");
  });

  it("shows a connect notice when GitHub is not connected", async () => {
    connected = false;
    render(page());
    expect(await screen.findByRole("button", { name: "Connect GitHub" })).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  });
});
