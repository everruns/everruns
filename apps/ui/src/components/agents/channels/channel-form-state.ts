import type {
  AgUiToolVisibility,
  ChannelAuthConfig,
  ChannelType,
  InvocationSessionMode,
  SessionStrategy,
  SlackResponsePolicy,
} from "@/lib/api/types";
import type { VoiceFormState } from "./voice-fields";
import type { ApiFormState } from "./api-fields";
import type { WorkChatMembership } from "./work-chat-fields";

export type ChannelFormState = {
  kind: ChannelType;
  chatInterface: "single_with_threads" | "multi_chat";
  chatMembership: WorkChatMembership;
  chatMemberEmail: string;
  chatInvites: string;
  enabled: boolean;
  slackSigningSecret: string;
  slackBotToken: string;
  slackTeamId: string;
  slackChannelId: string;
  slackCredentialsConfigured: boolean;
  /**
   * Which connected workspace one-click install creates the agent's app in. Transient: it is
   * sent to the install route, never saved into the channel's config.
   */
  slackInstallTeamId: string;
  /** Public id of the Slack app Everruns created for this channel, once there is one. */
  slackAppId: string;
  slackAgentSurfaceEnabled: boolean;
  slackCodeChannelsEnabled: boolean;
  slackSessionStrategy: SessionStrategy;
  slackResponsePolicy: SlackResponsePolicy;
  scheduleCronExpression: string;
  scheduleTimezone: string;
  invocationSessionMode: InvocationSessionMode;
  channelMessage: string;
  webhookToken: string;
  agUiToken: string;
  agUiAnonymous: boolean;
  agUiAuth?: ChannelAuthConfig;
  agUiAgentIdEnabled: boolean;
  agUiAgentIdClientId: string;
  agUiExpirationHours: number;
  agUiRateLimitPerMinute: string;
  agUiToolVisibility: AgUiToolVisibility;
  agUiGenericToolText: string;
  fcpToken: string;
  fcpAnonymous: boolean;
  fcpHandshake: string;
  fcpExpirationHours: number;
  fcpRateLimitPerMinute: string;
  fcpResponseTimeoutSeconds: string;
  publicChatAnonymous: boolean;
  publicChatToken: string;
  publicChatExpirationHours: number;
  publicChatRateLimitPerMinute: string;
  publicChatToolVisibility: AgUiToolVisibility;
  publicChatGenericToolText: string;
  publicChatDisplayName: string;
  publicChatLogoUrl: string;
  publicChatPrimaryColor: string;
  publicChatWelcomeMessage: string;
  publicChatCaptchaEnabled: boolean;
  publicChatTurnstileSiteKey: string;
  publicChatTurnstileSecretKey: string;
  publicChatGoogleEnabled: boolean;
  publicChatGoogleClientId: string;
  publicChatGoogleAllowedDomains: string;
  publicChatAgentIdEnabled: boolean;
  publicChatAgentIdClientId: string;
  voice: VoiceFormState;
  api: ApiFormState;
};
