import type { Agent, Harness } from "@/lib/api/legacy-api-types";
import { isPlatformChatThread, resolvePlatformChatIntro } from "@/lib/platform-chat-intro";

function harness(overrides: Partial<Harness> = {}): Harness {
  return {
    id: "h1",
    name: "platform-chat",
    display_name: "Platform Chat",
    icon: "everruns",
    description: "Harness description",
    system_prompt: "prompt",
    ...overrides,
  } as Harness;
}

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "a1",
    name: "ava",
    display_name: "Ava",
    description: "Agent description",
    system_prompt: "prompt",
    harness_id: "h1",
    ...overrides,
  } as Agent;
}

describe("isPlatformChatThread", () => {
  it("matches only the built-in Platform Chat harness", () => {
    expect(isPlatformChatThread("platform-chat")).toBe(true);
    expect(isPlatformChatThread("Platform Chat")).toBe(false);
    expect(isPlatformChatThread("research")).toBe(false);
    expect(isPlatformChatThread(null)).toBe(false);
    expect(isPlatformChatThread(undefined)).toBe(false);
  });
});

describe("resolvePlatformChatIntro", () => {
  it("returns nulls and no starters when neither side sets content", () => {
    expect(resolvePlatformChatIntro(agent(), harness())).toEqual({
      intro: null,
      description: null,
      starters: [],
    });
  });

  it("falls back to the harness when no agent is bound", () => {
    const resolved = resolvePlatformChatIntro(
      undefined,
      harness({
        intro_markdown: "# Hi",
        short_description: "Harness one-liner",
        starters: [{ icon: "zap", text: "Do it" }],
      }),
    );
    expect(resolved).toEqual({
      intro: "# Hi",
      description: "Harness one-liner",
      starters: [{ icon: "zap", text: "Do it" }],
    });
  });

  it("lets the agent win per field", () => {
    const resolved = resolvePlatformChatIntro(
      agent({ intro_markdown: "Agent intro" }),
      harness({ intro_markdown: "Harness intro", short_description: "Harness one-liner" }),
    );
    expect(resolved.intro).toBe("Agent intro");
    expect(resolved.description).toBe("Harness one-liner");
  });

  it("treats empty agent strings as unset", () => {
    const resolved = resolvePlatformChatIntro(
      agent({ intro_markdown: "", short_description: "" }),
      harness({ intro_markdown: "Harness intro", short_description: "Harness one-liner" }),
    );
    expect(resolved.intro).toBe("Harness intro");
    expect(resolved.description).toBe("Harness one-liner");
  });

  it("prefers agent starters only when non-empty", () => {
    const agentStarters = [{ icon: "bot", text: "Agent starter" }];
    expect(
      resolvePlatformChatIntro(
        agent({ starters: agentStarters }),
        harness({ starters: [{ text: "Harness starter" }] }),
      ).starters,
    ).toEqual(agentStarters);
    expect(
      resolvePlatformChatIntro(
        agent({ starters: [] }),
        harness({ starters: [{ text: "Harness starter" }] }),
      ).starters,
    ).toEqual([{ text: "Harness starter" }]);
  });
});
