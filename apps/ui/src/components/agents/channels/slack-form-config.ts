import type { ChannelFormState } from "./channel-form";

export function buildSlackChannelConfig(state: ChannelFormState) {
  return {
    ...(state.slackSigningSecret.trim() ? { signing_secret: state.slackSigningSecret.trim() } : {}),
    ...(state.slackBotToken.trim() ? { bot_token: state.slackBotToken.trim() } : {}),
    ...(state.slackTeamId.trim() ? { team_id: state.slackTeamId.trim() } : {}),
    ...(state.slackChannelId.trim() ? { channel_id: state.slackChannelId.trim() } : {}),
    agent_surface_enabled: state.slackAgentSurfaceEnabled,
    session_strategy: state.slackSessionStrategy,
    ...(state.slackResponsePolicy !== "all_messages"
      ? { response_policy: state.slackResponsePolicy }
      : {}),
  };
}
