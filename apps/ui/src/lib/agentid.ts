import type { ChannelAuthConfig } from "@/lib/api/types";

/** Issuer of AgentID, AgentMail's OpenID Connect provider for AI agents. */
export const AGENTID_ISSUER = "https://auth.agentid.com";

/**
 * The AgentID channel preset. Mirrors `ChannelAuthConfig::agentid_preset` on
 * the server: ordinary OIDC auth pinned to the AgentID issuer, keys from
 * AgentID discovery, the operator's client id as audience, and
 * `actor_type = "agent"`.
 */
export function agentIdChannelAuth(clientId: string): ChannelAuthConfig {
  return {
    mode: "oidc",
    provider: { type: "oidc", issuer: AGENTID_ISSUER },
    requirements: {
      audiences: [clientId.trim()],
      claims: { actor_type: "agent" },
    },
  };
}

/** Whether a channel auth config is the AgentID preset (discovered keys only). */
export function isAgentIdChannelAuth(auth: ChannelAuthConfig | undefined | null): boolean {
  return (
    auth?.mode === "oidc" &&
    auth.provider?.type === "oidc" &&
    auth.provider.issuer.replace(/\/+$/, "") === AGENTID_ISSUER &&
    !auth.provider.jwks_url
  );
}

/** The AgentID client id a preset config was saved with, or "". */
export function agentIdClientId(auth: ChannelAuthConfig | undefined | null): string {
  return isAgentIdChannelAuth(auth) ? (auth?.requirements?.audiences?.[0] ?? "") : "";
}
