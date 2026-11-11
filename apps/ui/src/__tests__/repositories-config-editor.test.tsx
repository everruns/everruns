import { fireEvent, render, screen } from "@testing-library/react";
import { CapabilityEditorContext } from "@/components/agents/capability-editor-context";
import { CapabilitySettingsEditor } from "@/components/agents/capability-settings-editor";
import type { Capability } from "@/lib/api/types";
import {
  allowedAccess,
  normalizeRepositoryName,
  readRepositoriesConfig,
  sessionRepositoriesConfig,
  writeRepositoriesConfig,
} from "@/lib/repositories-config";

let mockGitHub: { connected: boolean; identity_id: string | null } & Record<string, unknown>;
let mockReachable: Array<{ full_name: string; private: boolean; html_url: string }> | undefined;

jest.mock("@/hooks/use-agent-github", () => ({
  useAgentGitHub: () => ({ data: mockGitHub, isLoading: false }),
  useGitHubRepositories: () => ({ data: mockReachable }),
}));

// A native select stands in for the searchable popup, which has its own suite.
jest.mock("@/components/ui/combobox", () => ({
  Combobox: ({
    options,
    onValueChange,
    ariaLabel,
  }: {
    options: Array<{ value: string; label: string }>;
    onValueChange: (value: string) => void;
    ariaLabel: string;
  }) => (
    <select aria-label={ariaLabel} value="" onChange={(event) => onValueChange(event.target.value)}>
      <option value="">Add</option>
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  ),
}));

const capability = {
  id: "repositories",
  name: "Repositories",
  config_schema: { type: "object" },
} as unknown as Capability;

function renderEditor(config: Record<string, unknown>, onChange = jest.fn()) {
  render(
    <CapabilityEditorContext.Provider value={{ agentId: "agent_1" }}>
      <CapabilitySettingsEditor capability={capability} config={config} onChange={onChange} />
    </CapabilityEditorContext.Provider>,
  );
  return onChange;
}

describe("repositories config", () => {
  it("writes only what differs from the server's defaults and keeps unknown fields", () => {
    const config = readRepositoriesConfig({
      acts_as: "agent",
      repositories: [
        { repo: "acme/app", access: "write" },
        { repo: "acme/docs", ref: "main" },
      ],
      branch_prefix: "everruns/",
      depth: 200,
    });
    expect(writeRepositoriesConfig(config)).toEqual({
      acts_as: "agent",
      repositories: [
        { repo: "acme/app", access: "write" },
        { repo: "acme/docs", ref: "main" },
      ],
    });
  });

  it("accepts a GitHub URL as a repository name", () => {
    expect(normalizeRepositoryName(" https://github.com/acme/app.git ")).toBe("acme/app");
  });

  it("limits a session to what the agent allows", () => {
    const agent = readRepositoriesConfig({
      repositories: [{ repo: "acme/app", access: "write" }, { repo: "acme/docs" }],
    });
    expect(allowedAccess(agent, "acme/app")).toBe("write");
    expect(allowedAccess(agent, "Acme/Docs")).toBe("read");
    // Not listed: the default lets the agent add repositories to read.
    expect(allowedAccess(agent, "acme/other")).toBe("read");
    expect(allowedAccess({ ...agent, add_repositories: "off" }, "acme/other")).toBeUndefined();
    expect(allowedAccess({ ...agent, add_repositories: "write" }, "acme/other")).toBe("write");
  });

  it("gives a session its own config only when the set differs from the agent's", () => {
    const agentConfig = { repositories: [{ repo: "acme/app", access: "write" }], depth: 50 };
    expect(
      sessionRepositoriesConfig(agentConfig, [{ repo: "acme/app", access: "write" }]),
    ).toBeUndefined();
    expect(sessionRepositoriesConfig(agentConfig, [{ repo: "acme/app", access: "read" }])).toEqual({
      repositories: [{ repo: "acme/app" }],
      depth: 50,
    });
    // An emptied set is still a choice: the session works on no repository.
    expect(sessionRepositoriesConfig(agentConfig, [])).toEqual({ depth: 50 });
  });
});

describe("RepositoriesConfigEditor", () => {
  beforeEach(() => {
    mockGitHub = {
      connected: true,
      identity_id: "identity_1",
      kind: "everruns_app",
      account: "acme",
      missing_permissions: [],
    };
    mockReachable = [
      { full_name: "acme/app", private: true, html_url: "https://github.com/acme/app" },
      { full_name: "acme/docs", private: false, html_url: "https://github.com/acme/docs" },
    ];
  });

  it("adds a repository the agent's GitHub identity reaches, read only at first", () => {
    const onChange = renderEditor({ repositories: [{ repo: "acme/app", access: "write" }] });

    // Repositories already in the set are not offered again.
    expect(screen.queryByRole("option", { name: "acme/app" })).toBeNull();
    fireEvent.change(screen.getByRole("combobox", { name: "Add repository" }), {
      target: { value: "acme/docs" },
    });

    expect(onChange).toHaveBeenCalledWith({
      repositories: [{ repo: "acme/app", access: "write" }, { repo: "acme/docs" }],
    });
  });

  it("switches access, sets a branch and removes a repository", () => {
    const onChange = renderEditor({ repositories: [{ repo: "acme/app" }] });

    fireEvent.click(screen.getByRole("button", { name: "Write" }));
    expect(onChange).toHaveBeenLastCalledWith({
      repositories: [{ repo: "acme/app", access: "write" }],
    });

    fireEvent.change(screen.getByRole("textbox", { name: "Branch for acme/app" }), {
      target: { value: "develop" },
    });
    expect(onChange).toHaveBeenLastCalledWith({
      repositories: [{ repo: "acme/app", ref: "develop" }],
    });

    fireEvent.click(screen.getByRole("button", { name: "Remove acme/app" }));
    expect(onChange).toHaveBeenLastCalledWith({});
  });

  it("sets what the agent may add and the work branch prefix", () => {
    const onChange = renderEditor({});

    fireEvent.click(screen.getByRole("button", { name: "Off" }));
    expect(onChange).toHaveBeenLastCalledWith({ add_repositories: "off" });

    fireEvent.change(screen.getByRole("textbox", { name: "Work branch prefix" }), {
      target: { value: "bot/" },
    });
    expect(onChange).toHaveBeenLastCalledWith({ branch_prefix: "bot/" });
  });

  it("says who clones and pushes", () => {
    renderEditor({});
    expect(screen.getByText("Everruns GitHub App")).toBeInTheDocument();
    expect(screen.getByText(/installed on acme/)).toBeInTheDocument();
  });

  it("takes a typed name and points to GitHub setup when the agent has no identity", () => {
    mockGitHub = { connected: false, identity_id: null };
    mockReachable = undefined;
    const onChange = renderEditor({});

    expect(screen.getByRole("link", { name: "Connect GitHub" })).toHaveAttribute(
      "href",
      "/agents/agent_1/github",
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Add repository" }), {
      target: { value: "not a repository" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(screen.getByText("Enter a repository as owner/name.")).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.change(screen.getByRole("textbox", { name: "Add repository" }), {
      target: { value: "acme/app" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(onChange).toHaveBeenCalledWith({ repositories: [{ repo: "acme/app" }] });
  });
});
