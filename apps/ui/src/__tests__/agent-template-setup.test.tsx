import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AgentTemplateSetup } from "@/components/agents/agent-template-setup";
import {
  applyTemplateSettings,
  buildTemplateTrigger,
  canFinishSetup,
  importedExampleLanding,
} from "@/lib/agent-template-setup";
import type { AgentExampleSetup, GuidedAgentExample } from "@/lib/api/agent-examples";

const scannerSetup: AgentExampleSetup = {
  connect_github: true,
  connections: ["daytona"],
  repository_placeholder: "${repository}",
  trigger: {
    trigger_type: "schedule",
    cron_expression: "0 6 * * 1",
    session_mode: "session_per_invocation",
    message: "Run the scheduled security scan of ${repository} on its default branch.",
  },
  settings: [
    {
      key: "private_issues_only",
      label: "File findings on private repositories only",
      description: "d",
      capability: "github",
      config_key: "private_issues_only",
      default: true,
    },
    {
      key: "open_fix_pull_requests",
      label: "Open fix pull requests",
      description: "d",
      capability: "github",
      config_key: "allow_pull_requests",
      default: false,
    },
  ],
};

const reviewerSetup: AgentExampleSetup = {
  connect_github: true,
  connections: [],
  repository_placeholder: "${repository}",
  trigger: {
    trigger_type: "github",
    github_events: ["pull_request.opened", "pull_request.synchronize"],
    repositories: ["${repository}"],
    session_mode: "per_thread",
    message: "Pull request {{github.repository}}#{{github.number}} was {{github.action}}.",
  },
  settings: [],
};

const example = (name: string, setup?: AgentExampleSetup): GuidedAgentExample => ({
  harness_name: "bashkit-worker",
  name,
  display_name: name,
  description: `${name} description`,
  tags: [],
  capabilities: [],
  dev_only: false,
  setup,
});

describe("template setup helpers", () => {
  it("lands guided templates on their setup page and plain examples on the agent", () => {
    const examples = [example("pr-reviewer", reviewerSetup), example("dad-jokes-agent")];
    expect(importedExampleLanding(examples, "pr-reviewer", "agent_1")).toBe(
      "/agents/agent_1/setup?template=pr-reviewer",
    );
    expect(importedExampleLanding(examples, "dad-jokes-agent", "agent_1")).toBe("/agents/agent_1");
    expect(importedExampleLanding(undefined, "pr-reviewer", "agent_1")).toBe("/agents/agent_1");
  });

  it("fills the repository into the trigger but leaves runtime templates alone", () => {
    const github = buildTemplateTrigger(reviewerSetup, "acme/api");
    expect(github.repositories).toEqual(["acme/api"]);
    expect(github.message).toContain("{{github.repository}}");
    const schedule = buildTemplateTrigger(scannerSetup, "acme/api");
    expect(schedule.message).toBe(
      "Run the scheduled security scan of acme/api on its default branch.",
    );
    expect(reviewerSetup.trigger.repositories).toEqual(["${repository}"]);
  });

  it("writes settings into capability config only when they change", () => {
    const capabilities = [
      { ref: "github", config: { allow_pull_requests: false, private_issues_only: true } },
      { ref: "daytona", config: {} },
    ];
    expect(
      applyTemplateSettings(capabilities, scannerSetup.settings, {
        private_issues_only: true,
        open_fix_pull_requests: false,
      }),
    ).toBeNull();
    const next = applyTemplateSettings(capabilities, scannerSetup.settings, {
      private_issues_only: true,
      open_fix_pull_requests: true,
    });
    expect(next?.[0].config).toEqual({ allow_pull_requests: true, private_issues_only: true });
    expect(next?.[1]).toBe(capabilities[1]);
  });

  it("needs a connected App and an owner/name repository", () => {
    expect(canFinishSetup(reviewerSetup, false, "acme/api")).toBe(false);
    expect(canFinishSetup(reviewerSetup, true, "acme")).toBe(false);
    expect(canFinishSetup(reviewerSetup, true, "acme/api")).toBe(true);
  });
});

const push = jest.fn();
const createTrigger = jest.fn().mockResolvedValue({});
const updateAgent = jest.fn().mockResolvedValue({});
let connected = true;
let examples: GuidedAgentExample[] = [];

jest.mock("next/navigation", () => ({ useRouter: () => ({ push }) }));
jest.mock("@/hooks/use-agents", () => ({
  useAgent: () => ({
    data: {
      id: "agent_1",
      capabilities: [
        { ref: "github", config: { allow_pull_requests: false, private_issues_only: true } },
        { ref: "daytona", config: {} },
      ],
    },
  }),
  useUpdateAgent: () => ({ mutateAsync: updateAgent, isPending: false }),
}));
jest.mock("@/hooks/use-agent-examples", () => ({
  useAgentExamples: () => ({ data: examples, isLoading: false }),
}));
jest.mock("@/hooks/use-agent-triggers", () => ({
  useCreateAgentTrigger: () => ({ mutateAsync: createTrigger, isPending: false }),
}));
const connect = jest.fn();
jest.mock("@/hooks/use-agent-github", () => ({
  useAgentGitHub: () => ({
    data: { connected, app_created: connected, identity_id: "ident_1", account: "acme" },
  }),
  useConnectAgentGitHub: (_agentId: string, returnTo?: string) => ({
    mutate: () => connect(returnTo),
    isPending: false,
  }),
  useGitHubRepositories: () => ({
    data: connected ? [{ full_name: "acme/api", private: true, html_url: "u" }] : [],
  }),
}));

describe("AgentTemplateSetup", () => {
  beforeEach(() => {
    connected = true;
    examples = [example("pr-reviewer", reviewerSetup), example("security-scanner", scannerSetup)];
    jest.clearAllMocks();
  });

  it("creates the reviewer's GitHub trigger for the picked repository", async () => {
    render(<AgentTemplateSetup agentId="agent_1" templateName="pr-reviewer" />);
    fireEvent.click(await screen.findByRole("button", { name: "Create trigger" }));
    await waitFor(() => expect(createTrigger).toHaveBeenCalled());
    expect(createTrigger.mock.calls[0][0]).toMatchObject({
      trigger_type: "github",
      repositories: ["acme/api"],
      session_mode: "per_thread",
    });
    expect(updateAgent).not.toHaveBeenCalled();
    expect(push).toHaveBeenCalledWith("/agents/agent_1?tab=integrations");
  });

  it("keeps fix pull requests off unless switched on, then writes the capability config", async () => {
    render(<AgentTemplateSetup agentId="agent_1" templateName="security-scanner" />);
    const fix = await screen.findByRole("switch", { name: "Open fix pull requests" });
    expect(fix).not.toBeChecked();
    expect(screen.getByText(/daytona/)).toBeInTheDocument();
    fireEvent.click(fix);
    fireEvent.click(screen.getByRole("button", { name: "Create trigger" }));
    await waitFor(() => expect(createTrigger).toHaveBeenCalled());
    expect(updateAgent).toHaveBeenCalledWith({
      agentId: "agent_1",
      request: {
        capabilities: [
          { ref: "github", config: { allow_pull_requests: true, private_issues_only: true } },
          { ref: "daytona", config: {} },
        ],
      },
    });
    expect(createTrigger.mock.calls[0][0].message).toContain("acme/api");
  });

  it("asks to connect GitHub first and returns to the setup afterwards", async () => {
    connected = false;
    render(<AgentTemplateSetup agentId="agent_1" templateName="pr-reviewer" />);
    fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
    expect(connect).toHaveBeenCalledWith("/agents/agent_1/setup?template=pr-reviewer");
    expect(screen.getByRole("button", { name: "Create trigger" })).toBeDisabled();
  });

  it("explains when the template has no guided setup", () => {
    render(<AgentTemplateSetup agentId="agent_1" templateName="dad-jokes-agent" />);
    expect(screen.getByText(/no guided setup/)).toBeInTheDocument();
  });
});
