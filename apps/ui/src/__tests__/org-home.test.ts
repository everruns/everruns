import type { AttentionItem } from "@/lib/agents-home";
import type {
  AgentActivity,
  DoctorReport,
  HealthIssue,
  SandboxFleetStats,
  Session,
} from "@/lib/api/types";
import {
  busiestAgents,
  countOpenIssues,
  failureBursts,
  homeAttentionItems,
  hourlyTotals,
  rankAttention,
  type HomeAttentionItem,
} from "@/lib/org-home";

const NOW = Date.parse("2026-11-01T12:00:00Z");

function activity(
  id: string,
  hourly: Array<[number, number]>,
  extra: Partial<AgentActivity> = {},
): AgentActivity {
  return {
    agent_id: id,
    hourly: hourly.map(([runs, failed]) => ({ runs, failed })),
    runs: hourly.reduce((sum, [runs]) => sum + runs, 0),
    failed: hourly.reduce((sum, [, failed]) => sum + failed, 0),
    running_sessions: 0,
    last_turn_at: null,
    triggers: [],
    ...extra,
  } as AgentActivity;
}

function issue(overrides: Partial<HealthIssue>): HealthIssue {
  return {
    id: "issue-1",
    code: "slack.permissions",
    status: "open",
    title: "Slack is missing permissions",
    body: "Reconnect Slack.",
    href: "/settings/health?issue=issue-1",
    agent_id: null,
    agent_name: null,
    channel_id: null,
    first_detected_at: "2026-11-01T09:00:00Z",
    last_checked_at: "2026-11-01T11:00:00Z",
    missing_scopes: [],
    snoozed_until: null,
    ...overrides,
  } as HealthIssue;
}

const base = {
  agentItems: [] as AttentionItem[],
  healthIssues: [] as HealthIssue[],
  pausedSessions: [] as Session[],
  pausedTotal: 0,
  activity: [] as AgentActivity[],
  agentNames: new Map([["agent_a", "Release notes bot"]]),
  now: NOW,
};

describe("failureBursts", () => {
  it("flags an agent whose recent turns mostly failed", () => {
    const bursts = failureBursts([
      activity("agent_a", [
        [9, 0],
        [5, 4],
        [4, 3],
      ]),
    ]);
    expect(bursts).toEqual([{ agentId: "agent_a", runs: 9, failed: 7 }]);
  });

  it("ignores a single failure and old failures", () => {
    expect(
      failureBursts([
        activity("agent_a", [
          [0, 0],
          [1, 1],
        ]),
      ]),
    ).toEqual([]);
    expect(
      failureBursts([
        activity("agent_a", [
          [10, 10],
          [5, 0],
          [5, 1],
        ]),
      ]),
    ).toEqual([]);
  });
});

describe("homeAttentionItems", () => {
  it("returns nothing for a healthy organization", () => {
    expect(homeAttentionItems(base)).toEqual([]);
  });

  it("merges every source and puts blocking items first", () => {
    const items = homeAttentionItems({
      ...base,
      healthIssues: [
        issue({
          id: "org-limit",
          code: "org.active_turn_limit",
          title: "At the active-turn limit",
          href: "/settings/health?issue=org-limit",
          first_detected_at: "2026-11-01T11:48:00Z",
        }),
      ],
      pausedSessions: [
        { id: "session_1", title: "Q3 scan", updated_at: "2026-11-01T10:00:00Z" } as Session,
      ],
      pausedTotal: 1,
      activity: [
        activity("agent_a", [
          [5, 4],
          [4, 3],
        ]),
      ],
      sandboxStats: { needs_attention: 2 } as SandboxFleetStats,
      doctor: {
        checks: [
          {
            id: "workers",
            status: "problem",
            title: "No worker is online",
            summary: "Turns cannot run.",
            observed_at: "2026-11-01T11:00:00Z",
          },
          { id: "db", status: "ok", title: "Database", summary: "ok", observed_at: "" },
        ],
      } as unknown as DoctorReport,
    });
    expect(items.map((item) => item.key)).toEqual([
      "sessions:paused",
      "system:workers",
      "burst:agent_a",
      "health:org-limit",
      "sandboxes:attention",
    ]);
    expect(items.find((item) => item.key === "burst:agent_a")).toMatchObject({
      subject: "Release notes bot",
      values: { failed: 7, runs: 9 },
      href: "/sessions?status=failed&agent=agent_a",
    });
  });

  it("does not repeat a channel issue the agents model already reported", () => {
    const channelIssue = issue({ id: "slack", agent_id: "agent_a", agent_name: "Support" });
    const items = homeAttentionItems({
      ...base,
      healthIssues: [channelIssue],
      agentItems: [
        {
          key: "health:slack",
          kind: "permission",
          label: "Permission",
          title: channelIssue.title,
          body: channelIssue.body,
          agentId: "agent_a",
          agentName: "Support",
          severity: "warning",
          healthIssueId: "slack",
        },
      ],
    });
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      source: "agents",
      href: channelIssue.href,
      action: "Reconnect",
    });
  });

  it("leaves out snoozed and resolved issues", () => {
    const items = homeAttentionItems({
      ...base,
      healthIssues: [
        issue({ id: "snoozed", snoozed_until: "2026-11-01T13:00:00Z" }),
        issue({ id: "resolved", status: "resolved" }),
        issue({ id: "expired-snooze", snoozed_until: "2026-11-01T11:00:00Z" }),
      ],
    });
    expect(items.map((item) => item.key)).toEqual(["health:expired-snooze"]);
    expect(countOpenIssues([issue({ snoozed_until: "2026-11-01T13:00:00Z" })], NOW)).toBe(0);
  });
});

describe("rankAttention", () => {
  it("orders by severity, then by when the condition started", () => {
    const item = (key: string, severity: HomeAttentionItem["severity"], since?: string) =>
      ({ key, severity, since }) as HomeAttentionItem;
    const ranked = rankAttention([
      item("warning-old", "warning", "2026-10-30T00:00:00Z"),
      item("error-undated", "error"),
      item("error-new", "error", "2026-11-01T00:00:00Z"),
      item("error-old", "error", "2026-10-31T00:00:00Z"),
    ]);
    expect(ranked.map((entry) => entry.key)).toEqual([
      "error-old",
      "error-new",
      "error-undated",
      "warning-old",
    ]);
  });
});

describe("activity summaries", () => {
  it("sums hourly buckets across agents, aligned to the newest hour", () => {
    expect(
      hourlyTotals([
        activity("a", [
          [1, 0],
          [2, 1],
          [3, 0],
        ]),
        activity("b", [
          [4, 1],
          [5, 0],
        ]),
      ]),
    ).toEqual([
      { runs: 1, failed: 0 },
      { runs: 6, failed: 2 },
      { runs: 8, failed: 0 },
    ]);
  });

  it("lists running agents first, then the busiest", () => {
    const top = busiestAgents(
      [
        activity("idle", [[0, 0]]),
        activity("busy", [[40, 0]]),
        activity("live", [[2, 0]], { running_sessions: 1 }),
      ],
      2,
    );
    expect(top.map((agent) => agent.agent_id)).toEqual(["live", "busy"]);
  });
});
