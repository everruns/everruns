import { agentMembers } from "@/components/agents/agent-members-tab";
import type { Agent } from "@/lib/api/types";

function agent(capabilities: Agent["capabilities"]): Agent {
  return { id: "agent_lead", name: "lead", capabilities } as Agent;
}

describe("agentMembers", () => {
  it("is null without the Agents capability, so the tab stays hidden", () => {
    expect(agentMembers(agent([{ ref: "web_fetch", config: {} }]))).toBeNull();
  });

  it("reads members saved as targets or members and skips malformed entries", () => {
    expect(
      agentMembers(
        agent([
          {
            ref: "agent_handoff",
            config: { targets: [{ agent_id: "agent_a", id: "review" }, { id: "broken" }] },
          },
        ]),
      ),
    ).toEqual([{ agent_id: "agent_a", id: "review" }]);
    expect(
      agentMembers(
        agent([{ ref: "agent_handoff", config: { members: [{ agent_id: "agent_b" }] } }]),
      ),
    ).toEqual([{ agent_id: "agent_b" }]);
    expect(agentMembers(agent([{ ref: "agent_handoff", config: {} }]))).toEqual([]);
  });
});
