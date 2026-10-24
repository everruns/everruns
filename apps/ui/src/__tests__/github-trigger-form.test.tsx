import { ChatModeContext, type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import GitHubTriggerPage from "@/app/(main)/agents/[agentId]/triggers/[triggerId]/page";

let mockCanManage = true;
jest.mock("@/hooks/use-policies", () => ({ usePolicies: () => ({ can: () => mockCanManage }) }));

const create = jest.fn().mockResolvedValue({});
const push = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push }),
  usePathname: () => "/agents/agent_1/triggers/new",
  useSearchParams: () => new URLSearchParams(search),
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
let search = "type=github";
const connect = jest.fn();
jest.mock("@/hooks/use-agent-github", () => ({
  useAgentGitHub: () => ({
    data: {
      connected,
      app_created: connected,
      identity_id: "ident_1",
      suggested_owners: [{ login: "acme", kind: "organization" }],
    },
    isLoading: false,
  }),
  useConnectAgentGitHub: (_agentId: string, returnTo?: string) => ({
    mutateAsync: async (ownerOrg?: string) => connect(returnTo, ownerOrg),
    isPending: false,
  }),
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
    mockCanManage = true;
    connected = true;
    search = "type=github";
    create.mockClear();
    connect.mockClear();
  });

  it("sends changes_only when CI should only wake on a flip", async () => {
    render(page());
    expect(screen.queryByRole("checkbox", { name: "Only when the CI result changes" })).toBeNull();
    fireEvent.click(await screen.findByRole("checkbox", { name: "CI workflow finished" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Only when the CI result changes" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(create).toHaveBeenCalled());
    const request = create.mock.calls[0][0];
    expect(request.github_events).toContain("workflow_run.completed");
    expect(request.filter).toEqual({
      conditions: [],
      changes_only: { path: "github.ci.outcome" },
    });
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

  it("connects in a chosen organization and comes back to the prefilled trigger", async () => {
    connected = false;
    search = "type=github&events=workflow_run.completed";
    render(page());
    expect(screen.getByText(/before adding a GitHub trigger/)).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "acme" }));
    await waitFor(() =>
      expect(connect).toHaveBeenCalledWith(
        "/agents/agent_1/triggers/new?type=github&events=workflow_run.completed",
        "acme",
      ),
    );
    // The menu lives inside the trigger editor's form; opening it must not save.
    expect(create).not.toHaveBeenCalled();
  });

  it("types another organization inside the editor without submitting it", async () => {
    connected = false;
    render(page());
    fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Another organization…" }));
    const input = await screen.findByLabelText("GitHub organization");
    fireEvent.change(input, { target: { value: "beta-labs" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(connect).toHaveBeenCalledWith("/agents/agent_1/triggers/new?type=github", "beta-labs"),
    );
    expect(create).not.toHaveBeenCalled();
  });

  it("names the events the trigger listens to", () => {
    search = "type=github&events=workflow_run.completed";
    const { unmount } = render(page());
    expect(screen.getByText(/CI events/)).toBeInTheDocument();
    unmount();
    search = "type=github";
    render(page());
    expect(screen.getByText(/Pull request events/)).toBeInTheDocument();
  });
});

it("stages trigger text without changing conditions or saving, and enforces page permissions", () => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
  let adapter: PageDraftAdapter;
  const mode: ChatModeState = {
    page: { route: "/agents/agent_1/triggers/new", label: "New trigger" },
    pageContainer: { current: null },
    open: true,
    setOpen: () => {},
    actions: [],
    recordAction: () => {},
    registerPage: (_page, value) => {
      adapter = value!;
      return () => {};
    },
  };
  const { rerender } = render(
    <ChatModeContext.Provider value={mode}>{page()}</ChatModeContext.Provider>,
  );
  const before = adapter!.read(adapter!.binding!);
  act(() => {
    adapter!.patch(before.binding, before.revision, {
      message: "Review only completed CI failures.",
    });
  });
  expect(screen.getByDisplayValue("Review only completed CI failures.")).toBeInTheDocument();
  expect(create).not.toHaveBeenCalled();
  mockCanManage = false;
  rerender(<ChatModeContext.Provider value={mode}>{page()}</ChatModeContext.Provider>);
  const locked = adapter!.read(adapter!.binding!);
  expect(locked.writable).toBe(false);
  expect(() => adapter!.patch(locked.binding, locked.revision, { message: "Forbidden" })).toThrow(
    /read-only/,
  );
});
