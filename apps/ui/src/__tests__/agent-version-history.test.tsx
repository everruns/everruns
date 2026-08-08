import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AgentVersionHistory } from "@/components/agents/agent-version-history";
import type { Agent, AgentVersion } from "@/lib/api/types";

const push = jest.fn();
const createVersion = jest.fn();
const mockUseAgentVersionDiff = jest.fn(
  (_agentId: string, _fromVersionId?: string, _toVersionId?: string) => ({ data: undefined }),
);
let versions: AgentVersion[] = [];

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push }),
}));

jest.mock("@/hooks/use-agents", () => ({
  useAgentVersions: () => ({ data: versions, isLoading: false }),
  useCreateAgentVersion: () => ({ mutateAsync: createVersion, isPending: false }),
  useSetDefaultAgentVersion: () => ({ mutate: jest.fn(), isPending: false }),
  useRollbackAgentVersion: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useForkAgentVersion: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useAgentVersionDiff: (agentId: string, fromVersionId?: string, toVersionId?: string) =>
    mockUseAgentVersionDiff(agentId, fromVersionId, toVersionId),
}));

const agent = {
  id: "agt_test",
  default_version_id: null,
} as unknown as Agent;

function version(overrides: Partial<AgentVersion> = {}): AgentVersion {
  return {
    id: "ver_test",
    version: "draft.1",
    is_published: false,
    change_kind: "auto",
    summary: null,
    created_at: "2026-08-07T15:37:28Z",
    config_hash: "sha256:8b6be000000000000000",
    ...overrides,
  } as AgentVersion;
}

describe("AgentVersionHistory", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    versions = [];
    createVersion.mockResolvedValue(undefined);
  });

  it("prioritizes one clear action for an agent without saved versions", async () => {
    versions = [version()];

    render(<AgentVersionHistory agent={agent} />);

    expect(screen.getByText("Save your first version")).toBeInTheDocument();
    expect(screen.queryByText("Saved versions")).not.toBeInTheDocument();
    expect(screen.queryByText("Compare versions")).not.toBeInTheDocument();
    expect(screen.getByText("Recovery snapshots (1)").closest("details")).not.toHaveAttribute(
      "open",
    );
    expect(screen.getByText("Version options").closest("details")).not.toHaveAttribute("open");

    fireEvent.change(screen.getByLabelText("What changed? (optional)"), {
      target: { value: "Updated the instructions" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save current version" }));

    await waitFor(() =>
      expect(createVersion).toHaveBeenCalledWith({
        agentId: "agt_test",
        request: { summary: "Updated the instructions", change_kind: "manual" },
      }),
    );
  });

  it("keeps the compare action hidden until revisions are selected", () => {
    versions = [
      version({ id: "ver_2", version: "0.1.1", is_published: true, change_kind: "patch" }),
      version({ id: "ver_1", version: "0.1.0", is_published: true, change_kind: "manual" }),
    ];

    render(<AgentVersionHistory agent={agent} />);

    expect(screen.getByText("Save a new version")).toBeInTheDocument();
    expect(screen.getByText("Saved versions")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Compare versions"));
    expect(screen.getByRole("combobox", { name: "From revision" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "To revision" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Compare revisions" })).not.toBeInTheDocument();
    expect(mockUseAgentVersionDiff).toHaveBeenLastCalledWith("agt_test", undefined, undefined);
  });
});
