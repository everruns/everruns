import type {
  ChannelType,
  SlackConnection,
  SessionStrategy,
  SlackResponsePolicy,
  InvocationSessionMode,
  ChannelAuthConfig,
  AgUiToolVisibility,
} from "@/lib/api/types";
import type { EmailConfig } from "./email-fields";
import type { WorkChatMembership } from "./work-chat-fields";
import type { VoiceFormState } from "./voice-fields";
import type { ApiFormState } from "./api-fields";

export type ChannelFormState = {
  kind: ChannelType;
  email: EmailConfig;
  chatInterface: "single_with_threads" | "multi_chat";
  chatMembership: WorkChatMembership;
  chatViewPlacement: "none" | "panel" | "page";
  /** Work Chat: what each person may change about the plugins in their conversations. */
  chatPlugins: { allow_adding: boolean; allow_turning_off: boolean };
  chatMemberEmail: string;
  chatInvites: string;
  enabled: boolean;
  slackConnection: SlackConnection;
  slackAppToken: string;
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
  slackPrivateApprovals: boolean;
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
