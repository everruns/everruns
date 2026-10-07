import {
  buildChannelConfig,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/agents/channels/channel-form";
import { AGENTID_ISSUER, agentIdChannelAuth, isAgentIdChannelAuth } from "@/lib/agentid";
import type { AgentChannel } from "@/lib/api/types";

const preset = {
  mode: "oidc",
  provider: { type: "oidc", issuer: AGENTID_ISSUER },
  requirements: {
    audiences: ["agentid-client"],
    claims: { actor_type: "agent" },
  },
};

describe("AgentID channel preset", () => {
  it("writes ordinary OIDC auth pinned to AgentID with the agent claim", () => {
    expect(agentIdChannelAuth(" agentid-client ")).toEqual(preset);
  });

  it("treats a custom JWKS URL as not AgentID", () => {
    expect(isAgentIdChannelAuth(agentIdChannelAuth("c"))).toBe(true);
    expect(
      isAgentIdChannelAuth({
        mode: "oidc",
        provider: {
          type: "oidc",
          issuer: AGENTID_ISSUER,
          jwks_url: "https://x.example/jwks",
        },
      }),
    ).toBe(false);
  });

  it("public chat offers AgentID beside Google and needs a client id", () => {
    const state = {
      ...getDefaultChannelFormState("public_chat"),
      publicChatAnonymous: false,
      publicChatAgentIdEnabled: true,
      publicChatAgentIdClientId: "",
    };
    expect(isChannelFormValid(state)).toBe(false);

    const filled = { ...state, publicChatAgentIdClientId: "agentid-client" };
    expect(isChannelFormValid(filled)).toBe(true);
    expect(buildChannelConfig(filled)).toMatchObject({ auth: preset });
  });

  it("round-trips an AG-UI channel saved with the preset", () => {
    const channel = {
      channel_type: "ag_ui",
      enabled: true,
      channel_config: { anonymous: false },
      auth: preset,
    } as unknown as AgentChannel;
    const state = getDefaultChannelFormState("ag_ui", channel);
    expect(state.agUiAgentIdEnabled).toBe(true);
    expect(state.agUiAgentIdClientId).toBe("agentid-client");
    expect(buildChannelConfig(state)).toMatchObject({ auth: preset });

    const off = { ...state, agUiAgentIdEnabled: false };
    expect(buildChannelConfig(off)).not.toHaveProperty("auth");
  });
});
