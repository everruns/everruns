import {
  draftCapabilityConfigs,
  draftChannelRequests,
  draftGitHubTrigger,
  draftMcpAttachments,
  draftScheduleTrigger,
  githubTriggerSetupHref,
} from "@/components/agents/new/create-from-draft";
import { githubFormFromParams } from "@/components/agents/github-trigger-form";
import { EMPTY_DRAFT, type AgentDraft } from "@/lib/api/agent-draft";
import type { Capability } from "@/lib/api/types";
import {
  DEFAULT_WEBHOOK_MESSAGE,
  draftBlocker,
  isValidAgentSlug,
  newAgentLanding,
  parseNewAgentTab,
  slugifyAgentName,
} from "@/lib/new-agent";

const draft = (patch: Partial<AgentDraft>): AgentDraft => ({
  ...EMPTY_DRAFT,
  ...patch,
});

describe("new agent page", () => {
  it("defaults to describing the agent", () => {
    expect(parseNewAgentTab(null)).toBe("describe");
    expect(parseNewAgentTab("bogus")).toBe("describe");
    expect(parseNewAgentTab("import")).toBe("import");
  });

  it("derives a slug the server accepts", () => {
    expect(slugifyAgentName("  Invoice Chaser!! ")).toBe("invoice-chaser");
    expect(slugifyAgentName("a".repeat(70) + " b")).toHaveLength(64);
    expect(isValidAgentSlug("invoice-chaser")).toBe(true);
    expect(isValidAgentSlug("-bad")).toBe(false);
    expect(isValidAgentSlug("Bad")).toBe(false);
  });

  it("says what still blocks creating the draft", () => {
    expect(draftBlocker(EMPTY_DRAFT)).toBe("Give the agent a name");
    expect(draftBlocker(draft({ display_name: "X", name: "x" }))).toBe("Write the instructions");
    expect(
      draftBlocker(draft({ display_name: "X", name: "x", system_prompt: "Do it" })),
    ).toBeNull();
  });

  it("creates ways in as draft channels, the schedule as a trigger, and leaves Slack to its own form", () => {
    const requests = draftChannelRequests(
      draft({
        channels: ["public_chat", "slack", "webhook"],
        schedule: {
          cron: "0 9 * * 1-5",
          timezone: "Europe/Kyiv",
          message: "Chase invoices",
        },
      }),
    );
    expect(requests.map((r) => r.channel_type)).toEqual(["public_chat", "webhook"]);
    expect(requests[1].channel_config).toMatchObject({
      message: DEFAULT_WEBHOOK_MESSAGE,
    });
    expect(
      draftScheduleTrigger(
        draft({ schedule: { cron: "0 9 * * 1-5", timezone: "Europe/Kyiv", message: "Chase" } }),
      ),
    ).toEqual({
      trigger_type: "schedule",
      cron_expression: "0 9 * * 1-5",
      timezone: "Europe/Kyiv",
      session_mode: "shared_session",
      message: "Chase",
    });
    expect(draftScheduleTrigger(EMPTY_DRAFT)).toBeNull();
    expect(newAgentLanding("agent_1", draft({ channels: ["slack"] }))).toBe(
      "/agents/agent_1/channels/new?kind=slack",
    );
    expect(newAgentLanding("agent_1", draft({ channels: ["webhook"] }))).toBe("/agents/agent_1");
    expect(newAgentLanding("agent_1", draft({}), { testRun: true })).toBe(
      "/playground/new?agent=agent_1&dry_run=1",
    );
  });

  it("attaches capability dependencies first, once", () => {
    const capabilities = [
      { id: "web_search", dependencies: ["web_fetch"] },
      { id: "web_fetch", dependencies: [] },
    ] as unknown as Capability[];
    expect(draftCapabilityConfigs(["web_search", "web_fetch"], capabilities)).toEqual([
      { ref: "web_fetch", config: {} },
      { ref: "web_search", config: {} },
    ]);
  });

  it("turns a CI watcher draft into a GitHub trigger, catalog attachments and a prefilled form", () => {
    const watcher = draft({
      github_trigger: {
        events: ["workflow_run.completed"],
        repositories: ["everruns/everruns"],
        changes_only: true,
        message: "Main CI is {{github.ci.outcome}}",
      },
      mcp_servers: [
        { name: "visti", acts_as: "none" },
        { name: "github", acts_as: "service" },
      ],
    });
    expect(draftGitHubTrigger(watcher)).toEqual({
      trigger_type: "github",
      github_events: ["workflow_run.completed"],
      repositories: ["everruns/everruns"],
      session_mode: "session_per_invocation",
      message: "Main CI is {{github.ci.outcome}}",
      enabled: true,
      filter: { conditions: [], changes_only: { path: "github.ci.outcome" } },
    });
    expect(draftMcpAttachments(watcher)).toEqual({
      visti: { use: "catalog:visti", actsAs: "none" },
      github: { use: "catalog:github", actsAs: "service" },
    });
    expect(draftMcpAttachments(EMPTY_DRAFT)).toBeUndefined();
    expect(draftGitHubTrigger(EMPTY_DRAFT)).toBeNull();

    const href = githubTriggerSetupHref("agent_1", watcher)!;
    expect(href.startsWith("/agents/agent_1/triggers/new?type=github&")).toBe(true);
    const form = githubFormFromParams(new URL(href, "http://x").searchParams);
    expect(form.events).toEqual(["workflow_run.completed"]);
    expect(form.repositories).toEqual(["everruns/everruns"]);
    expect(form.session_mode).toBe("session_per_invocation");
    expect(form.message).toBe("Main CI is {{github.ci.outcome}}");
    expect(form.filter?.changes_only?.path).toBe("github.ci.outcome");
  });

  it("keeps pull request triggers per thread and ignores unknown events in the URL", () => {
    const form = githubFormFromParams(
      new URLSearchParams({ events: "pull_request.opened,push", message: "Review" }),
    );
    expect(form.events).toEqual(["pull_request.opened"]);
    expect(form.session_mode).toBe("per_thread");
    expect(form.filter?.changes_only).toBeUndefined();
    expect(githubFormFromParams(new URLSearchParams()).events.length).toBeGreaterThan(0);
  });
});
