import {
  agentNow,
  attentionItems,
  channelState,
  matchesAgentTab,
  sortChannels,
} from "@/lib/agents-home";
import type { OrgExposure } from "@/hooks/use-org-exposures";
import type { Agent, AgentActivity, AgentChannel, HealthIssue } from "@/lib/api/types";

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "agent_a",
    name: "support-triage",
    display_name: "Support triage",
    status: "active",
    exposures_suspended: false,
    ...overrides,
  } as Agent;
}

function exposure(overrides: Partial<OrgExposure> = {}): OrgExposure {
  return {
    channel: { id: "appchan_a", channel_type: "public_chat" } as AgentChannel,
    agent: agent(),
    state: "live",
    anonymous: false,
    publiclyReachable: false,
    isTrigger: false,
    lastInvokedAt: null,
    ...overrides,
  };
}

function activity(overrides: Partial<AgentActivity> = {}): AgentActivity {
  return {
    agent_id: "agent_a",
    running_sessions: 0,
    last_turn_at: null,
    runs: 0,
    failed: 0,
    hourly: [],
    triggers: [],
    ...overrides,
  } as AgentActivity;
}

function issue(overrides: Partial<HealthIssue> = {}): HealthIssue {
  return {
    id: "issue_1",
    code: "slack.permissions",
    agent_id: "agent_a",
    agent_name: "Support triage",
    channel_id: "appchan_slack",
    status: "open",
    title: "Slack bot is missing reactions:write",
    body: "Reconnect Slack to grant the required permissions.",
    missing_scopes: ["reactions:write"],
    first_detected_at: "2026-10-09T00:00:00Z",
    last_checked_at: new Date().toISOString(),
    stale: false,
    ...overrides,
  } as HealthIssue;
}

describe("channel words", () => {
  it.each([
    ["live", "live"],
    ["draft", "draft"],
    ["disabled", "paused"],
    ["suspended", "paused"],
    ["agent-inactive", "agent-archived"],
  ] as const)("maps %s to %s", (state, expected) => {
    expect(channelState({ state })).toBe(expected);
  });

  it("puts open doors first, then live channels by traffic", () => {
    const quiet = exposure({ channel: { id: "quiet" } as AgentChannel });
    const busy = exposure({ channel: { id: "busy" } as AgentChannel });
    const draft = exposure({ channel: { id: "draft" } as AgentChannel, state: "draft" });
    const open = exposure({
      channel: { id: "open" } as AgentChannel,
      anonymous: true,
      publiclyReachable: true,
    });
    const sorted = sortChannels([draft, quiet, busy, open], new Map([["busy", 9]]));
    expect(sorted.map((e) => e.channel.id)).toEqual(["open", "busy", "quiet", "draft"]);
  });
});

describe("needs attention", () => {
  const models = new Map([
    ["model_off", { enabled: false, display_name: "gpt-4.1" }],
    ["model_on", { enabled: true, display_name: "gpt-6" }],
  ]);

  it("lists setup problems builders can fix, errors before warnings", () => {
    const items = attentionItems({
      agents: [
        agent({ id: "agent_a", default_model_id: "model_off" } as Partial<Agent>),
        agent({ id: "agent_b", default_model_id: "model_on" } as Partial<Agent>),
      ],
      exposures: [exposure({ anonymous: true, publiclyReachable: true })],
      healthIssues: [issue()],
      models,
    });
    expect(items.map((item) => item.kind)).toEqual(["model", "public-access", "permission"]);
    expect(items[2].severity).toBe("warning");
    expect(items[2].healthIssueId).toBe("issue_1");
  });

  it("leaves out capacity issues, resolved issues and archived agents", () => {
    const items = attentionItems({
      agents: [agent({ status: "archived", default_model_id: "model_off" } as Partial<Agent>)],
      exposures: [],
      healthIssues: [
        issue({ agent_id: null, code: "org.active_turn_limit" }),
        issue({ id: "resolved", status: "resolved" }),
        issue({ id: "archived-agent" }),
      ],
      models,
    });
    expect(items).toEqual([]);
  });

  it("does not report a model it cannot see", () => {
    const items = attentionItems({
      agents: [agent({ default_model_id: "unknown" } as Partial<Agent>)],
      exposures: [],
      healthIssues: [],
      models,
    });
    expect(items).toEqual([]);
  });

  it("treats a broken connection with no missing scope as an error", () => {
    const [item] = attentionItems({
      agents: [agent()],
      exposures: [],
      healthIssues: [issue({ missing_scopes: [] })],
      models,
    });
    expect(item.kind).toBe("connection");
    expect(item.severity).toBe("error");
  });
});

describe("agent now", () => {
  it("prefers archived, then paused channels, then running", () => {
    expect(agentNow(agent({ status: "archived" }), activity()).label).toBe("Archived");
    expect(
      agentNow(agent({ exposures_suspended: true }), activity({ running_sessions: 2 })).label,
    ).toBe("Channels paused");
    expect(agentNow(agent(), activity({ running_sessions: 3 })).label).toBe("3 running");
    expect(agentNow(agent(), undefined).label).toBe("Never run");
    expect(agentNow(agent(), activity({ last_turn_at: new Date().toISOString() })).label).toMatch(
      /^Idle · last run/,
    );
  });

  it("filters tabs on the same facts the rows show", () => {
    const running = activity({ running_sessions: 1 });
    expect(matchesAgentTab("running", agent(), running, false)).toBe(true);
    expect(matchesAgentTab("idle", agent(), running, false)).toBe(false);
    expect(matchesAgentTab("attention", agent(), undefined, true)).toBe(true);
    expect(matchesAgentTab("all", agent({ status: "archived" }), undefined, false)).toBe(false);
    expect(matchesAgentTab("archived", agent({ status: "archived" }), undefined, false)).toBe(true);
  });
});
