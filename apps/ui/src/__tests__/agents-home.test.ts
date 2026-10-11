import {
  adoptedExample,
  agentChannelHref,
  agentNow,
  attentionItems,
  channelState,
  channelTrafficLine,
  fastestFirstReply,
  formatReplyTime,
  matchesAgentTab,
  sortChannels,
} from "@/lib/agents-home";
import type { OrgExposure } from "@/hooks/use-org-exposures";
import type {
  Agent,
  AgentActivity,
  AgentChannel,
  ChannelActivity,
  HealthIssue,
} from "@/lib/api/types";

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

describe("agent channel link", () => {
  it("opens that channel on its agent", () => {
    expect(agentChannelHref("agent_dj", "appchan_slack")).toBe(
      "/agents/agent_dj?tab=integrations&channel=appchan_slack",
    );
  });
});

describe("channel words", () => {
  it.each([
    ["live", "live"],
    ["draft", "draft"],
    ["disabled", "draft"],
    ["suspended", "paused"],
    ["agent-inactive", "agent-archived"],
  ] as const)("maps %s to %s", (state, expected) => {
    expect(channelState({ state })).toBe(expected);
  });

  it("puts open doors first, then live channels by traffic", () => {
    const quiet = exposure({ channel: { id: "quiet" } as AgentChannel });
    const busy = exposure({ channel: { id: "busy" } as AgentChannel });
    const draft = exposure({
      channel: { id: "draft" } as AgentChannel,
      state: "draft",
    });
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
        agent({
          id: "agent_a",
          default_model_id: "model_off",
        } as Partial<Agent>),
        agent({
          id: "agent_b",
          default_model_id: "model_on",
        } as Partial<Agent>),
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
      agents: [
        agent({
          status: "archived",
          default_model_id: "model_off",
        } as Partial<Agent>),
      ],
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

function channelActivity(overrides: Partial<ChannelActivity> = {}): ChannelActivity {
  return {
    channel_id: "appchan_a",
    sessions: 0,
    daily: [],
    last_session_at: null,
    people: 0,
    identified_sessions: 0,
    median_first_reply_ms: null,
    ...overrides,
  } as ChannelActivity;
}

describe("channel audience", () => {
  it("shows people only when the channel can tell callers apart", () => {
    expect(channelTrafficLine(undefined)).toBe("No traffic in 7d");
    expect(channelTrafficLine(channelActivity({ sessions: 5 }))).toBe("5 sessions · 7d");
    expect(
      channelTrafficLine(channelActivity({ sessions: 5, people: 1, identified_sessions: 5 })),
    ).toBe("5 sessions · 1 person · 7d");
  });

  it("formats reply times precisely when fast and coarsely when slow", () => {
    expect(formatReplyTime(820)).toBe("820 ms");
    expect(formatReplyTime(4200)).toBe("4.2 s");
    expect(formatReplyTime(42_000)).toBe("42 s");
    expect(formatReplyTime(180_000)).toBe("3 min");
  });

  it("picks the live channel with the quickest median reply", () => {
    const slack = exposure({
      channel: { id: "appchan_slack", channel_type: "slack" } as AgentChannel,
    });
    const chat = exposure();
    const quiet = exposure({
      channel: { id: "appchan_quiet", channel_type: "webhook" } as AgentChannel,
    });
    const activity = new Map([
      ["appchan_slack", channelActivity({ median_first_reply_ms: 9000 })],
      ["appchan_a", channelActivity({ median_first_reply_ms: 3000 })],
    ]);
    expect(fastestFirstReply([slack, chat, quiet], activity)).toEqual({
      exposure: chat,
      ms: 3000,
    });
    expect(fastestFirstReply([quiet], activity)).toBeNull();
  });
});

describe("setup not finished", () => {
  const guided = new Map([["pr-reviewer", { display_name: "PR Reviewer" }]]);
  const adopted = agent({
    tags: ["github", "template", "example:pr-reviewer"],
    created_at: "2026-10-09T00:00:00Z",
  });
  const inputs = {
    exposures: [],
    healthIssues: [],
    models: new Map(),
    guidedExamples: guided,
  };

  it("reads the adopted example from the agent's tags", () => {
    expect(adoptedExample(adopted)).toBe("pr-reviewer");
    expect(adoptedExample(agent({ tags: ["template"] }))).toBeNull();
  });

  it("flags a guided example with no trigger", () => {
    const items = attentionItems({
      ...inputs,
      agents: [adopted],
      activity: new Map(),
    });
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      kind: "setup",
      exampleName: "pr-reviewer",
    });
  });

  it("stays quiet once the trigger exists, for plain examples, and before activity loads", () => {
    const withTrigger = new Map([
      ["agent_a", activity({ triggers: [{ trigger_type: "github", enabled: true }] })],
    ]);
    expect(attentionItems({ ...inputs, agents: [adopted], activity: withTrigger })).toEqual([]);
    const plain = agent({ tags: ["example:researcher"] });
    expect(attentionItems({ ...inputs, agents: [plain], activity: new Map() })).toEqual([]);
    expect(attentionItems({ ...inputs, agents: [adopted] })).toEqual([]);
  });
});
