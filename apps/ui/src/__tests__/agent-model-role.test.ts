import {
  RESEARCH_MODEL_OPTION,
  agentModelId,
  agentModelRequest,
  agentModelValue,
} from "@/lib/agent-model-role";

describe("agent model role", () => {
  it("shows a research-role agent as the research option", () => {
    expect(agentModelValue({ default_model_id: null, model_role: "research" })).toBe(
      RESEARCH_MODEL_OPTION,
    );
    expect(agentModelValue({ default_model_id: "model-1" })).toBe("model-1");
    expect(agentModelValue({})).toBe("");
  });

  it("sends the role, a pinned model, or a cleared role", () => {
    expect(agentModelRequest(RESEARCH_MODEL_OPTION, "model-1")).toEqual({
      model_role: "research",
    });
    expect(agentModelRequest("model-2", RESEARCH_MODEL_OPTION)).toEqual({
      default_model_id: "model-2",
    });
    expect(agentModelRequest(undefined, RESEARCH_MODEL_OPTION)).toEqual({
      default_model_id: undefined,
      model_role: null,
    });
    expect(agentModelRequest("model-1", "model-1")).toEqual({ default_model_id: "model-1" });
  });

  it("resolves the research option to the organization's research model", () => {
    const org = { effective_research_model_id: "fable" };
    expect(agentModelId(RESEARCH_MODEL_OPTION, org)).toBe("fable");
    expect(agentModelId("model-1", org)).toBe("model-1");
    expect(agentModelId(RESEARCH_MODEL_OPTION, undefined)).toBe("");
  });
});
