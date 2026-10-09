import {
  draftCapabilityConfigs,
  draftChannelRequests,
  draftScheduleTrigger,
} from "@/components/agents/new/create-from-draft";
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
});
