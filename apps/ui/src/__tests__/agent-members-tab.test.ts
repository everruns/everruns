import { agentMembers, memberFiles, withFiles } from "@/components/agents/agent-members-tab";
import type { Agent } from "@/lib/api/types";

function agent(capabilities: Agent["capabilities"]): Agent {
  return { id: "agent_lead", name: "lead", capabilities } as Agent;
}

describe("agentMembers", () => {
  it("is null without Spawn agents or Threads, so the tab stays hidden", () => {
    expect(agentMembers(agent([{ ref: "web_fetch", config: {} }]))).toBeNull();
  });

  it("reads agents from Spawn agents or Threads and skips malformed entries", () => {
    expect(
      agentMembers(
        agent([
          {
            ref: "spawn_agents",
            config: {
              self: true,
              agents: [{ agent_id: "agent_a", id: "review" }, { id: "broken" }],
            },
          },
        ]),
      ),
    ).toEqual([{ agent_id: "agent_a", id: "review" }]);
    expect(
      agentMembers(agent([{ ref: "threads", config: { agents: [{ agent_id: "agent_c" }] } }])),
    ).toEqual([{ agent_id: "agent_c" }]);
    expect(agentMembers(agent([{ ref: "spawn_agents", config: {} }]))).toEqual([]);
  });

  it("still reads lists saved as targets or members", () => {
    expect(
      agentMembers(
        agent([
          {
            ref: "spawn_agents",
            config: { targets: [{ agent_id: "agent_a", id: "review" }, { id: "broken" }] },
          },
        ]),
      ),
    ).toEqual([{ agent_id: "agent_a", id: "review" }]);
    expect(
      agentMembers(agent([{ ref: "threads", config: { members: [{ agent_id: "agent_b" }] } }])),
    ).toEqual([{ agent_id: "agent_b" }]);
  });
});

describe("memberFiles", () => {
  it("defaults to the member's own files and reads a copy", () => {
    expect(memberFiles({ agent_id: "a" })).toBe("own");
    expect(memberFiles({ agent_id: "a", files: "copy" })).toBe("copy");
    expect(memberFiles({ agent_id: "a", files: "same" })).toBe("own");
  });

  it("drops the setting when set back to own, so saved config stays minimal", () => {
    expect(withFiles({ agent_id: "a", id: "review" }, "copy")).toEqual({
      agent_id: "a",
      id: "review",
      files: "copy",
    });
    expect(withFiles({ agent_id: "a", files: "copy" }, "own")).toEqual({ agent_id: "a" });
  });
});
