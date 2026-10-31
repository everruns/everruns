"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useCallback, useId, useState } from "react";
import { ChevronDown, RefreshCw } from "lucide-react";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { buildSlackChannelConfig } from "./slack-form-config";
import { Textarea } from "@/components/ui/textarea";
import {
  SlackConnectionStatus,
  SlackManualSetup,
  type SlackChannelSetupConfig,
} from "@/components/agents/integrations/slack-setup-guidance";
import {
  SlackConversationSettings,
  SlackAgentPaneSettings,
} from "@/components/apps/slack-conversation-settings";
import { CronInput, isSupportedCronExpression } from "@/components/apps/cron-label";
import {
  DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
  DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS,
} from "@/lib/api/types";
import type {
  AgUiChannelConfig,
  AgUiToolVisibility,
  ChannelAuthConfig,
  AgentChannel,
  ChannelType,
  FcpChannelConfig,
  InvocationSessionMode,
  PublicChatChannelConfig,
  ScheduleChannelConfig,
  VoiceChannelConfig,
  SessionStrategy,
  SlackResponsePolicy,
  WebhookChannelConfig,
} from "@/lib/api/types";
import { agentIdChannelAuth, agentIdClientId, isAgentIdChannelAuth } from "@/lib/agentid";
import { AgentIdSignInFields, PublicChatSignInFields } from "./sign-in-fields";
import {
  buildVoiceChannelConfig,
  isVoiceFormValid,
  VoiceFields,
  voiceFormStateFromConfig,
  type VoiceFormState,
} from "./voice-fields";
import {
  ApiFields,
  apiFormStateFromConfig,
  buildApiChannelConfig,
  isApiFormValid,
  type ApiChannelConfig,
  type ApiFormState,
} from "./api-fields";
import {
  getAgUiToolVisibilityDisplayName,
  getInvocationSessionModeDisplayName,
} from "@/lib/channel-display";
import { generateChannelToken } from "@/lib/channel-tokens";
import { slackAppUrl } from "@/components/slack/slack-workspaces";
import { SlackWorkspaceChoice, useSlackInstall } from "./slack-install";
import type { SlackInstallCapability } from "@/lib/api/agent-channels";
import { SlackInstallError } from "@/components/slack/slack-install-error";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

import { WorkChatFields } from "./work-chat-fields";
import { FoldedSettings, FormSection } from "./form-section";

export const CHANNEL_FORM_KINDS: ChannelType[] = [
  "schedule",
  "api",
  "work_chat",
  "webhook",
  "ag_ui",
  "public_chat",
  "voice",
  "fcp",
  "slack",
];

const DEFAULT_PUBLIC_CHAT_EXPIRATION_HOURS = 6;

const DEFAULT_FCP_EXPIRATION_HOURS = 6;
const DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS = 120;

export type ChannelFormState = {
  kind: ChannelType;
  chatInterface: "single_with_threads" | "multi_chat";
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

function secretValue(value?: string, configured?: boolean): string {
  if (value) return value;
  return configured ? "" : "";
}

export function getDefaultChannelFormState(
  kind: ChannelType,
  channel?: AgentChannel,
): ChannelFormState {
  const base: ChannelFormState = {
    kind,
    enabled: channel?.enabled ?? true,
    slackSigningSecret: "",
    slackBotToken: "",
    slackTeamId: "",
    slackChannelId: "",
    slackCredentialsConfigured: false,
    slackInstallTeamId: "",
    slackAppId: "",
    slackAgentSurfaceEnabled: false,
    slackSessionStrategy: "per_thread",
    slackResponsePolicy: "all_messages",
    scheduleCronExpression: "0 0 * * * * *",
    scheduleTimezone: "UTC",
    invocationSessionMode: "shared_session",
    channelMessage: "",
    webhookToken: "",
    agUiToken: kind === "ag_ui" && !channel ? generateChannelToken() : "",
    agUiAnonymous: true,
    agUiAuth: undefined,
    agUiAgentIdEnabled: false,
    agUiAgentIdClientId: "",
    agUiExpirationHours: DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS / 3600,
    agUiRateLimitPerMinute: "",
    agUiToolVisibility: "generic",
    agUiGenericToolText: DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
    fcpToken: "",
    fcpAnonymous: true,
    fcpHandshake: "",
    fcpExpirationHours: DEFAULT_FCP_EXPIRATION_HOURS,
    fcpRateLimitPerMinute: "",
    fcpResponseTimeoutSeconds: String(DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS),
    publicChatAnonymous: true,
    publicChatToken: "",
    publicChatExpirationHours: DEFAULT_PUBLIC_CHAT_EXPIRATION_HOURS,
    publicChatRateLimitPerMinute: "",
    publicChatToolVisibility: "generic",
    publicChatGenericToolText: DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
    publicChatDisplayName: "",
    publicChatLogoUrl: "",
    publicChatPrimaryColor: "",
    publicChatWelcomeMessage: "",
    publicChatCaptchaEnabled: false,
    publicChatTurnstileSiteKey: "",
    publicChatTurnstileSecretKey: "",
    publicChatGoogleEnabled: false,
    publicChatGoogleClientId: "",
    publicChatGoogleAllowedDomains: "",
    publicChatAgentIdEnabled: false,
    publicChatAgentIdClientId: "",
    voice: voiceFormStateFromConfig(),
    api: apiFormStateFromConfig(),
    chatInterface: "multi_chat",
  };

  if (!channel) return base;

  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return {
      ...base,
      kind: "schedule",
      scheduleCronExpression: config.cron_expression || base.scheduleCronExpression,
      scheduleTimezone: config.timezone || "UTC",
      invocationSessionMode: config.session_mode || "shared_session",
      channelMessage: config.message || "",
    };
  }
  if (channel.channel_type === "webhook") {
    const config = channel.channel_config as WebhookChannelConfig;
    return {
      ...base,
      kind: "webhook",
      webhookToken: secretValue(config.token, config.token_configured),
      invocationSessionMode: config.session_mode || "shared_session",
      channelMessage: config.message || "",
    };
  }
  if (channel.channel_type === "ag_ui") {
    const config = channel.channel_config as AgUiChannelConfig;
    const auth = channel.auth ?? config.auth;
    return {
      ...base,
      kind: "ag_ui",
      agUiToken: secretValue(config.token, config.token_configured),
      agUiAnonymous: config.anonymous ?? true,
      agUiAuth: auth,
      agUiAgentIdEnabled: isAgentIdChannelAuth(auth),
      agUiAgentIdClientId: agentIdClientId(auth),
      agUiExpirationHours:
        typeof config.session_expiration_seconds === "number"
          ? config.session_expiration_seconds / 3600
          : base.agUiExpirationHours,
      agUiRateLimitPerMinute:
        typeof config.rate_limit_per_minute === "number"
          ? String(config.rate_limit_per_minute)
          : "",
      agUiToolVisibility: config.tool_visibility || "generic",
      agUiGenericToolText: config.generic_tool_text || DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
    };
  }
  if (channel.channel_type === "fcp") {
    const config = channel.channel_config as FcpChannelConfig;
    return {
      ...base,
      kind: "fcp",
      fcpToken: secretValue(config.token, config.token_configured),
      fcpAnonymous: config.anonymous ?? true,
      fcpHandshake: config.handshake ?? "",
      fcpExpirationHours:
        typeof config.session_expiration_seconds === "number"
          ? config.session_expiration_seconds / 3600
          : base.fcpExpirationHours,
      fcpRateLimitPerMinute:
        typeof config.rate_limit_per_minute === "number"
          ? String(config.rate_limit_per_minute)
          : "",
      fcpResponseTimeoutSeconds:
        typeof config.response_timeout_seconds === "number"
          ? String(config.response_timeout_seconds)
          : String(DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS),
    };
  }
  if (channel.channel_type === "public_chat") {
    const config = channel.channel_config as PublicChatChannelConfig;
    const auth = channel.auth ?? config.auth;
    return {
      ...base,
      kind: "public_chat",
      publicChatAnonymous: config.anonymous ?? true,
      publicChatToken: secretValue(config.token, config.token_configured),
      publicChatExpirationHours:
        typeof config.session_expiration_seconds === "number"
          ? config.session_expiration_seconds / 3600
          : base.publicChatExpirationHours,
      publicChatRateLimitPerMinute:
        typeof config.rate_limit_per_minute === "number"
          ? String(config.rate_limit_per_minute)
          : "",
      publicChatToolVisibility: config.tool_visibility || "generic",
      publicChatGenericToolText: config.generic_tool_text || DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
      publicChatDisplayName: config.branding?.display_name ?? "",
      publicChatLogoUrl: config.branding?.logo_url ?? "",
      publicChatPrimaryColor: config.branding?.primary_color ?? "",
      publicChatWelcomeMessage: config.branding?.welcome_message ?? "",
      publicChatCaptchaEnabled: config.captcha?.enabled ?? false,
      publicChatTurnstileSiteKey: config.captcha?.site_key ?? "",
      // Secret key is write-only and never returned; always start blank.
      publicChatTurnstileSecretKey: "",
      publicChatGoogleEnabled: auth?.provider?.type === "google_oidc",
      publicChatGoogleClientId:
        auth?.provider?.type === "google_oidc" ? auth.provider.client_id : "",
      publicChatGoogleAllowedDomains:
        auth?.provider?.type === "google_oidc"
          ? (auth.provider.allowed_domains ?? []).join(", ")
          : "",
      publicChatAgentIdEnabled: isAgentIdChannelAuth(auth),
      publicChatAgentIdClientId: agentIdClientId(auth),
    };
  }
  if (channel.channel_type === "work_chat") {
    const config = channel.channel_config as {
      interface_mode?: "single_with_threads" | "multi_chat";
      execution?: ApiChannelConfig;
    };
    return {
      ...base,
      kind: "work_chat",
      chatInterface: config.interface_mode ?? "multi_chat",
      api: apiFormStateFromConfig(config.execution),
    };
  }
  if (channel.channel_type === "api") {
    const config = channel.channel_config as ApiChannelConfig;
    return { ...base, kind: "api", api: apiFormStateFromConfig(config) };
  }
  if (channel.channel_type === "voice") {
    const config = channel.channel_config as VoiceChannelConfig;
    return { ...base, kind: "voice", voice: voiceFormStateFromConfig(config) };
  }
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelSetupConfig;
    return {
      ...base,
      kind: "slack",
      slackSigningSecret: secretValue(config.signing_secret, config.signing_secret_configured),
      slackBotToken: secretValue(config.bot_token, config.bot_token_configured),
      slackTeamId: config.team_id || "",
      slackChannelId: config.channel_id || "",
      slackCredentialsConfigured: Boolean(
        config.signing_secret_configured || config.bot_token_configured,
      ),
      slackAppId: config.slack_app_id || "",
      slackAgentSurfaceEnabled: config.agent_surface_enabled ?? false,
      slackSessionStrategy: config.session_strategy || "per_thread",
      slackResponsePolicy: config.response_policy || "all_messages",
    };
  }
  return { ...base, kind: channel.channel_type };
}

export function buildChannelConfig(state: ChannelFormState) {
  switch (state.kind) {
    case "schedule":
      return {
        cron_expression: state.scheduleCronExpression,
        timezone: state.scheduleTimezone || "UTC",
        session_mode: state.invocationSessionMode,
        message: state.channelMessage,
      };
    case "webhook":
      return {
        ...(state.webhookToken.trim() ? { token: state.webhookToken.trim() } : {}),
        session_mode: state.invocationSessionMode,
        message: state.channelMessage,
      };
    case "ag_ui": {
      const rateLimit = Number.parseInt(state.agUiRateLimitPerMinute, 10);
      // AgentID replaces whatever auth the channel had; turning it off keeps a
      // non-AgentID config that was set elsewhere (for example through the API).
      const agUiAuth = state.agUiAgentIdEnabled
        ? agentIdChannelAuth(state.agUiAgentIdClientId)
        : isAgentIdChannelAuth(state.agUiAuth)
          ? undefined
          : state.agUiAuth;
      return {
        anonymous: state.agUiAnonymous,
        ...(agUiAuth ? { auth: agUiAuth } : {}),
        ...(state.agUiToken.trim() ? { token: state.agUiToken.trim() } : {}),
        session_expiration_seconds: Math.max(0, Math.round(state.agUiExpirationHours * 3600)),
        ...(Number.isFinite(rateLimit) && rateLimit > 0
          ? { rate_limit_per_minute: rateLimit }
          : {}),
        tool_visibility: state.agUiToolVisibility,
        generic_tool_text: state.agUiGenericToolText.trim() || DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
      };
    }
    case "fcp": {
      const rateLimit = Number.parseInt(state.fcpRateLimitPerMinute, 10);
      const timeout = Number.parseInt(state.fcpResponseTimeoutSeconds, 10);
      return {
        anonymous: state.fcpAnonymous,
        ...(state.fcpToken.trim() ? { token: state.fcpToken.trim() } : {}),
        ...(state.fcpHandshake.trim() ? { handshake: state.fcpHandshake } : {}),
        session_expiration_seconds: Math.max(0, Math.round(state.fcpExpirationHours * 3600)),
        ...(Number.isFinite(rateLimit) && rateLimit > 0
          ? { rate_limit_per_minute: rateLimit }
          : {}),
        response_timeout_seconds:
          Number.isFinite(timeout) && timeout > 0 && timeout <= 600
            ? timeout
            : DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS,
      };
    }
    case "public_chat": {
      const rateLimit = Number.parseInt(state.publicChatRateLimitPerMinute, 10);
      const branding = {
        ...(state.publicChatDisplayName.trim()
          ? { display_name: state.publicChatDisplayName.trim() }
          : {}),
        ...(state.publicChatLogoUrl.trim() ? { logo_url: state.publicChatLogoUrl.trim() } : {}),
        ...(state.publicChatPrimaryColor.trim()
          ? { primary_color: state.publicChatPrimaryColor.trim() }
          : {}),
        ...(state.publicChatWelcomeMessage.trim()
          ? { welcome_message: state.publicChatWelcomeMessage.trim() }
          : {}),
      };
      const captcha = state.publicChatTurnstileSiteKey.trim()
        ? {
            provider: "turnstile" as const,
            enabled: state.publicChatCaptchaEnabled,
            site_key: state.publicChatTurnstileSiteKey.trim(),
            ...(state.publicChatTurnstileSecretKey.trim()
              ? { secret_key: state.publicChatTurnstileSecretKey.trim() }
              : {}),
          }
        : undefined;
      const allowedDomains = state.publicChatGoogleAllowedDomains
        .split(",")
        .map((d) => d.trim())
        .filter(Boolean);
      const auth =
        state.publicChatGoogleEnabled && state.publicChatGoogleClientId.trim()
          ? {
              mode: "google_oidc" as const,
              provider: {
                type: "google_oidc" as const,
                client_id: state.publicChatGoogleClientId.trim(),
                ...(allowedDomains.length > 0 ? { allowed_domains: allowedDomains } : {}),
              },
            }
          : state.publicChatAgentIdEnabled && state.publicChatAgentIdClientId.trim()
            ? agentIdChannelAuth(state.publicChatAgentIdClientId)
            : undefined;
      return {
        anonymous: state.publicChatAnonymous,
        ...(state.publicChatToken.trim() ? { token: state.publicChatToken.trim() } : {}),
        session_expiration_seconds: Math.max(0, Math.round(state.publicChatExpirationHours * 3600)),
        ...(Number.isFinite(rateLimit) && rateLimit > 0
          ? { rate_limit_per_minute: rateLimit }
          : {}),
        tool_visibility: state.publicChatToolVisibility,
        generic_tool_text:
          state.publicChatGenericToolText.trim() || DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
        ...(Object.keys(branding).length > 0 ? { branding } : {}),
        ...(captcha ? { captcha } : {}),
        ...(auth ? { auth } : {}),
      };
    }
    case "voice":
      return buildVoiceChannelConfig(state.voice);
    case "work_chat":
      return {
        interface_mode: state.chatInterface,
        execution: { ...buildApiChannelConfig(state.api), errors: "public", visibility: "full" },
      };
    case "api":
      return buildApiChannelConfig(state.api);
    case "slack":
      return buildSlackChannelConfig(state);
    default:
      return {};
  }
}

export function isChannelFormValid(state: ChannelFormState): boolean {
  if (!CHANNEL_FORM_KINDS.includes(state.kind)) return false;
  if (state.kind === "schedule") {
    return isSupportedCronExpression(state.scheduleCronExpression) && !!state.channelMessage.trim();
  }
  if (state.kind === "webhook") return !!state.channelMessage.trim();
  if (state.kind === "ag_ui") {
    if (state.agUiAgentIdEnabled && !state.agUiAgentIdClientId.trim()) return false;
    return state.agUiToolVisibility !== "generic" || state.agUiGenericToolText.trim().length <= 120;
  }
  if (state.kind === "fcp") {
    // anonymous=false without a token is allowed by the server (operator
    // may intend to fill it in later), but the UI nudges the user to set
    // one — the form is still valid either way.
    const timeout = Number.parseInt(state.fcpResponseTimeoutSeconds, 10);
    if (!Number.isFinite(timeout) || timeout <= 0 || timeout > 600) return false;
    return true;
  }
  if (state.kind === "public_chat") {
    // The backend caps generic_tool_text regardless of tool visibility.
    if (state.publicChatGenericToolText.trim().length > 120) {
      return false;
    }
    // Captcha needs a site key when enabled or when a secret is being set.
    if (
      (state.publicChatCaptchaEnabled || state.publicChatTurnstileSecretKey.trim()) &&
      !state.publicChatTurnstileSiteKey.trim()
    ) {
      return false;
    }
    // Google sign-in needs a client ID when enabled.
    if (state.publicChatGoogleEnabled && !state.publicChatGoogleClientId.trim()) {
      return false;
    }
    if (state.publicChatAgentIdEnabled && !state.publicChatAgentIdClientId.trim()) {
      return false;
    }
    // Turning anonymous access off requires a sign-in provider; otherwise the
    // server forbids every request and the chat is unreachable. (A shared token
    // only gates the anonymous path, so it cannot substitute here.)
    if (
      !state.publicChatAnonymous &&
      !state.publicChatGoogleEnabled &&
      !state.publicChatAgentIdEnabled
    ) {
      return false;
    }
    return true;
  }
  if (state.kind === "voice") return isVoiceFormValid(state.voice);
  if (state.kind === "api" || state.kind === "work_chat") return isApiFormValid(state.api);
  if (state.kind === "slack") return true;
  return false;
}

function FieldGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid gap-4 md:grid-cols-2">{children}</div>;
}

export function ChannelForm({
  state,
  onChange,
  mode,
  channelId,
  channel,
  slackInstallDisabled = false,
  slackInstallCapability,
  onSlackCapabilityChanged,
}: {
  state: ChannelFormState;
  onChange: (state: ChannelFormState) => void;
  mode: "new" | "edit";
  /**
   * The channel's public id (`appchan_…`), present once it exists. One-click
   * Slack install needs it because Slack redirects back to this channel's own
   * callback route, so the button appears only after the channel is saved.
   */
  channelId?: string;
  channel?: AgentChannel;
  slackInstallDisabled?: boolean;
  slackInstallCapability?: SlackInstallCapability;
  onSlackCapabilityChanged?: () => void | Promise<unknown>;
}) {
  const ui = useUiTranslations();
  const update = <K extends keyof ChannelFormState>(key: K, value: ChannelFormState[K]) =>
    onChange({ ...state, [key]: value });
  const slackInstallAvailable = slackInstallCapability?.connected === true;
  const slackFormId = useId();
  const [manualSlackOpen, setManualSlackOpen] = useState(false);
  const openManualSlack = useCallback(() => setManualSlackOpen(true), []);
  const slackInstall = useSlackInstall(channelId, state.slackInstallTeamId, openManualSlack);
  // The OAuth exchange is what records the workspace id, so an app id plus a workspace id means
  // Everruns created this channel's app and it was installed.
  const slackLive = Boolean(state.slackAppId && state.slackTeamId);
  const selectSlackInstallTeam = useCallback(
    (teamId: string) => onChange({ ...state, slackInstallTeamId: teamId }),
    [onChange, state],
  );

  return (
    <div>
      {state.kind === "schedule" && (
        <>
          <FormSection title="When it runs">
            <CronInput
              value={state.scheduleCronExpression}
              timezone={state.scheduleTimezone}
              onChange={(value) => update("scheduleCronExpression", value)}
              onTimezoneChange={(value) => update("scheduleTimezone", value)}
            />
          </FormSection>
          <FormSection
            title="What it sends"
            hint="The message the agent receives each time this runs."
          >
            <div className="space-y-2">
              <Label htmlFor="channel_message">{ui("Invocation message")}</Label>
              <Textarea
                id="channel_message"
                value={state.channelMessage}
                onChange={(event) => update("channelMessage", event.target.value)}
                placeholder={
                  state.kind === "schedule"
                    ? ui("Run {{app.name}} now.")
                    : ui("Process webhook payload for {{payload.repo.name}}.")
                }
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="session_mode">{ui("Session mode")}</Label>
              <Select
                value={state.invocationSessionMode}
                onValueChange={(value) =>
                  update("invocationSessionMode", value as InvocationSessionMode)
                }
              >
                <SelectTrigger id="session_mode">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared_session">
                    {getInvocationSessionModeDisplayName("shared_session")}
                  </SelectItem>
                  <SelectItem value="session_per_invocation">
                    {getInvocationSessionModeDisplayName("session_per_invocation")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </FormSection>
        </>
      )}
      {state.kind === "webhook" && (
        <>
          <FormSection
            title="What it sends"
            hint="The message the agent receives for each request. It can use fields from the request body."
          >
            <div className="space-y-2">
              <Label htmlFor="channel_message">{ui("Invocation message")}</Label>
              <Textarea
                id="channel_message"
                value={state.channelMessage}
                onChange={(event) => update("channelMessage", event.target.value)}
                placeholder={ui("Process webhook payload for {{payload.repo.name}}.")}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="session_mode">{ui("Session mode")}</Label>
              <Select
                value={state.invocationSessionMode}
                onValueChange={(value) =>
                  update("invocationSessionMode", value as InvocationSessionMode)
                }
              >
                <SelectTrigger id="session_mode">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared_session">
                    {getInvocationSessionModeDisplayName("shared_session")}
                  </SelectItem>
                  <SelectItem value="session_per_invocation">
                    {getInvocationSessionModeDisplayName("session_per_invocation")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </FormSection>
          <FormSection title="Access" hint="Requests without this token are refused.">
            <div className="space-y-2">
              <Label htmlFor="webhook_token">{ui("Webhook token")}</Label>
              <Input
                id="webhook_token"
                type="password"
                value={state.webhookToken}
                onChange={(event) => update("webhookToken", event.target.value)}
                placeholder={
                  mode === "edit" ? ui("Leave blank to keep existing token") : ui("shared-secret")
                }
              />
              <p className="text-xs text-muted-foreground">
                {ui("Send as Authorization: Bearer <token> or X-Everruns-Webhook-Token.")}
              </p>
            </div>
          </FormSection>
        </>
      )}
      {state.kind === "ag_ui" && (
        <>
          <FormSection title="Access" hint="Your web app sends this token with every request.">
            <div className="space-y-2">
              <Label htmlFor="ag_ui_token">{ui("Bearer token")}</Label>
              <div className="flex gap-2">
                <Input
                  id="ag_ui_token"
                  value={state.agUiToken}
                  onChange={(event) => update("agUiToken", event.target.value)}
                  className="font-mono"
                  placeholder={
                    mode === "edit"
                      ? ui("Leave blank to keep existing token")
                      : ui("Generated bearer token")
                  }
                />
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  onClick={() => update("agUiToken", generateChannelToken())}
                  aria-label={ui("Regenerate AG-UI token")}
                >
                  <RefreshCw className="size-4" />
                </Button>
              </div>
            </div>
            <AgentIdSignInFields
              idPrefix="ag_ui"
              description={ui(
                "Accept AgentID tokens from AI agents. When on, every request must present a valid AgentID token for your client and the bearer token above is not used.",
              )}
              enabled={state.agUiAgentIdEnabled}
              clientId={state.agUiAgentIdClientId}
              onEnabledChange={(checked) => update("agUiAgentIdEnabled", checked)}
              onClientIdChange={(value) => update("agUiAgentIdClientId", value)}
            />
          </FormSection>
          <FormSection
            title="What visitors see"
            hint="Tool names, arguments and results never reach visitors."
          >
            <div className="space-y-2">
              <Label htmlFor="ag_ui_tool_visibility">{ui("Tool visibility")}</Label>
              <Select
                value={state.agUiToolVisibility}
                onValueChange={(value) => update("agUiToolVisibility", value as AgUiToolVisibility)}
              >
                <SelectTrigger id="ag_ui_tool_visibility">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">{getAgUiToolVisibilityDisplayName("none")}</SelectItem>
                  <SelectItem value="generic">
                    {getAgUiToolVisibilityDisplayName("generic")}
                  </SelectItem>
                  <SelectItem value="narrated">
                    {getAgUiToolVisibilityDisplayName("narrated")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            {state.agUiToolVisibility === "generic" && (
              <div className="space-y-2">
                <Label htmlFor="ag_ui_generic_tool_text">{ui("Generic activity text")}</Label>
                <Input
                  id="ag_ui_generic_tool_text"
                  value={state.agUiGenericToolText}
                  onChange={(event) => update("agUiGenericToolText", event.target.value)}
                  maxLength={120}
                />
                <p className="text-xs text-muted-foreground">
                  {ui("Tool names, arguments, and results are never sent to public clients.")}
                </p>
              </div>
            )}
          </FormSection>
          <FormSection title="Limits">
            <div className="space-y-2">
              <Label htmlFor="ag_ui_rate_limit">{ui("Rate limit / minute")}</Label>
              <Input
                id="ag_ui_rate_limit"
                value={state.agUiRateLimitPerMinute}
                onChange={(event) => update("agUiRateLimitPerMinute", event.target.value)}
                inputMode="numeric"
                placeholder={ui("No per-channel cap")}
              />
            </div>
          </FormSection>
        </>
      )}
      {state.kind === "fcp" && (
        <>
          <FormSection title="Access">
            <div className="flex items-center justify-between border p-3">
              <div>
                <p className="text-sm font-medium">{ui("Anonymous access")}</p>
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "When off, every request must present the bearer token below. The FCP handshake (GET on the channel URL) stays public so clients can discover how to authenticate.",
                  )}
                </p>
              </div>
              <Switch
                checked={state.fcpAnonymous}
                onCheckedChange={(checked) => update("fcpAnonymous", checked)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="fcp_token">{ui("Bearer token")}</Label>
              <div className="flex gap-2">
                <Input
                  id="fcp_token"
                  value={state.fcpToken}
                  onChange={(event) => update("fcpToken", event.target.value)}
                  className="font-mono"
                  placeholder={
                    mode === "edit"
                      ? ui("Leave blank to keep existing token")
                      : ui("Generated bearer token")
                  }
                />
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  onClick={() => update("fcpToken", generateChannelToken())}
                  aria-label={ui("Regenerate FCP token")}
                >
                  <RefreshCw className="size-4" />
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                {ui(
                  "Verified with constant-time comparison; sent as Authorization: Bearer <token> or X-Everruns-FCP-Token. Never echoed in responses.",
                )}
              </p>
            </div>
          </FormSection>
          <FormSection
            title="Greeting"
            hint="What a client reads first, to learn how to talk to the agent."
          >
            <div className="space-y-2">
              <Label htmlFor="fcp_handshake">{ui("Custom handshake (Markdown)")}</Label>
              <Textarea
                id="fcp_handshake"
                value={state.fcpHandshake}
                onChange={(event) => update("fcpHandshake", event.target.value)}
                placeholder={ui(
                  "Leave blank to auto-generate from the agent name and description.",
                )}
                rows={4}
              />
              <p className="text-xs text-muted-foreground">
                {ui("Returned verbatim for GET on the channel URL. Use plain Markdown.")}
              </p>
            </div>
          </FormSection>
          <FormSection title="Limits">
            <FoldedSettings
              summary={limitsSummary(state.fcpRateLimitPerMinute, state.fcpExpirationHours, ui)}
            >
              <FieldGrid>
                <div className="space-y-2">
                  <Label htmlFor="fcp_expiration">{ui("Session expiration (hours)")}</Label>
                  <Input
                    id="fcp_expiration"
                    value={String(state.fcpExpirationHours)}
                    onChange={(event) => {
                      const parsed = Number.parseFloat(event.target.value);
                      update("fcpExpirationHours", Number.isFinite(parsed) ? parsed : 0);
                    }}
                    inputMode="numeric"
                    placeholder="6"
                  />
                  <p className="text-xs text-muted-foreground">
                    {ui("0 = never expire the fcp_session cookie.")}
                  </p>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="fcp_rate_limit">{ui("Rate limit / minute")}</Label>
                  <Input
                    id="fcp_rate_limit"
                    value={state.fcpRateLimitPerMinute}
                    onChange={(event) => update("fcpRateLimitPerMinute", event.target.value)}
                    inputMode="numeric"
                    placeholder={ui("No per-channel cap")}
                  />
                  <p className="text-xs text-muted-foreground">
                    {ui("FCP-only limiter namespace; cannot share buckets with other channels.")}
                  </p>
                </div>
              </FieldGrid>
              <div className="space-y-2">
                <Label htmlFor="fcp_response_timeout">{ui("Response timeout (seconds)")}</Label>
                <Input
                  id="fcp_response_timeout"
                  value={state.fcpResponseTimeoutSeconds}
                  onChange={(event) => update("fcpResponseTimeoutSeconds", event.target.value)}
                  inputMode="numeric"
                  placeholder="120"
                />
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "How long the channel waits for the agent before returning 504. Must be 1-600s.",
                  )}
                </p>
              </div>
            </FoldedSettings>
          </FormSection>
        </>
      )}
      {state.kind === "public_chat" && (
        <>
          <FormSection title="Who can chat" hint="Changing this does not change the link.">
            <div role="radiogroup" aria-label={ui("Who can chat")} className="border">
              <label
                className={cn(
                  "grid cursor-pointer grid-cols-[1rem_minmax(0,1fr)] gap-3 p-3.5",
                  state.publicChatAnonymous && "bg-accent/10",
                )}
              >
                <input
                  type="radio"
                  name="public_chat_access"
                  className="mt-1 size-4 accent-primary"
                  checked={state.publicChatAnonymous}
                  onChange={() => update("publicChatAnonymous", true)}
                />
                <span>
                  <span className="block text-sm font-medium">{ui("Anyone with the link")}</span>
                  <span className="block text-sm text-muted-foreground">
                    {ui("No sign-in. Good for a demo or a public help page.")}
                  </span>
                </span>
              </label>
              {state.publicChatAnonymous && (
                <div className="px-3.5 pb-3.5 pl-10">
                  <div className="space-y-2">
                    <Label htmlFor="public_chat_token">{ui("Shared token (optional)")}</Label>
                    <div className="flex gap-2">
                      <Input
                        id="public_chat_token"
                        value={state.publicChatToken}
                        onChange={(event) => update("publicChatToken", event.target.value)}
                        className="font-mono"
                        placeholder={
                          mode === "edit"
                            ? ui("Leave blank to keep existing token")
                            : ui("Optional bearer token")
                        }
                      />
                      <Button
                        type="button"
                        variant="outline"
                        size="icon"
                        onClick={() => update("publicChatToken", generateChannelToken())}
                        aria-label={ui("Regenerate Public Chat token")}
                      >
                        <RefreshCw className="size-4" />
                      </Button>
                    </div>
                  </div>
                </div>
              )}
              <label
                className={cn(
                  "grid cursor-pointer grid-cols-[1rem_minmax(0,1fr)] gap-3 border-t border-border/60 p-3.5",
                  !state.publicChatAnonymous && "bg-accent/10",
                )}
              >
                <input
                  type="radio"
                  name="public_chat_access"
                  className="mt-1 size-4 accent-primary"
                  checked={!state.publicChatAnonymous}
                  onChange={() => update("publicChatAnonymous", false)}
                />
                <span>
                  <span className="block text-sm font-medium">{ui("Only people who sign in")}</span>
                  <span className="block text-sm text-muted-foreground">
                    {ui(
                      "Visitors sign in first, with Google or AgentID. Turn on at least one below.",
                    )}
                  </span>
                </span>
              </label>
              {!state.publicChatAnonymous && (
                <div className="space-y-4 px-3.5 pb-3.5 pl-10">
                  <PublicChatSignInFields
                    googleEnabled={state.publicChatGoogleEnabled}
                    googleClientId={state.publicChatGoogleClientId}
                    googleAllowedDomains={state.publicChatGoogleAllowedDomains}
                    agentIdEnabled={state.publicChatAgentIdEnabled}
                    agentIdClientId={state.publicChatAgentIdClientId}
                    update={update}
                  />
                </div>
              )}
            </div>
            <div className="space-y-3">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <Label htmlFor="public_chat_captcha">
                    {ui("Check for bots before the first message")}
                  </Label>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {ui(
                      "Visitors who are not signed in pass a Cloudflare Turnstile check first. Signed-in visitors skip it.",
                    )}
                  </p>
                </div>
                <Switch
                  id="public_chat_captcha"
                  checked={state.publicChatCaptchaEnabled}
                  onCheckedChange={(checked) => update("publicChatCaptchaEnabled", checked)}
                />
              </div>
              {(state.publicChatCaptchaEnabled || state.publicChatTurnstileSiteKey) && (
                <>
                  <FieldGrid>
                    <div className="space-y-2">
                      <Label htmlFor="public_chat_turnstile_site_key">
                        {ui("Turnstile site key")}
                      </Label>
                      <Input
                        id="public_chat_turnstile_site_key"
                        value={state.publicChatTurnstileSiteKey}
                        onChange={(event) =>
                          update("publicChatTurnstileSiteKey", event.target.value)
                        }
                        className="font-mono"
                        placeholder={ui("0x4AAAAAAA...")}
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="public_chat_turnstile_secret_key">
                        {ui("Turnstile secret key")}
                      </Label>
                      <Input
                        id="public_chat_turnstile_secret_key"
                        type="password"
                        value={state.publicChatTurnstileSecretKey}
                        onChange={(event) =>
                          update("publicChatTurnstileSecretKey", event.target.value)
                        }
                        className="font-mono"
                        placeholder={
                          mode === "edit"
                            ? ui("Leave blank to keep existing secret")
                            : ui("0x4AAAAAAA...")
                        }
                      />
                    </div>
                  </FieldGrid>
                  <p className="text-xs text-muted-foreground">
                    {ui("The secret key is write-only and never shown after saving.")}
                  </p>
                </>
              )}
            </div>
          </FormSection>
          <FormSection
            title="Look and feel"
            hint="All optional. Empty fields use the agent's name."
          >
            <>
              <FieldGrid>
                <div className="space-y-2">
                  <Label htmlFor="public_chat_display_name">{ui("Display name")}</Label>
                  <Input
                    id="public_chat_display_name"
                    value={state.publicChatDisplayName}
                    onChange={(event) => update("publicChatDisplayName", event.target.value)}
                    maxLength={120}
                    placeholder={ui("Falls back to the agent name")}
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="public_chat_primary_color">{ui("Main color")}</Label>
                  <Input
                    id="public_chat_primary_color"
                    value={state.publicChatPrimaryColor}
                    onChange={(event) => update("publicChatPrimaryColor", event.target.value)}
                    placeholder="#0A1636"
                  />
                </div>
              </FieldGrid>
              <div className="space-y-2">
                <Label htmlFor="public_chat_logo_url">{ui("Logo URL")}</Label>
                <Input
                  id="public_chat_logo_url"
                  value={state.publicChatLogoUrl}
                  onChange={(event) => update("publicChatLogoUrl", event.target.value)}
                  placeholder="https://example.com/logo.png"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="public_chat_welcome">{ui("Welcome message")}</Label>
                <Textarea
                  id="public_chat_welcome"
                  value={state.publicChatWelcomeMessage}
                  onChange={(event) => update("publicChatWelcomeMessage", event.target.value)}
                  placeholder={ui("Shown before the visitor's first message.")}
                  rows={2}
                />
              </div>
            </>
          </FormSection>
          <FormSection title="Limits" hint="Defaults suit most pages.">
            <FoldedSettings
              summary={limitsSummary(
                state.publicChatRateLimitPerMinute,
                state.publicChatExpirationHours,
                ui,
              )}
            >
              <FieldGrid>
                <div className="space-y-2">
                  <Label htmlFor="public_chat_tool_visibility">{ui("Tool visibility")}</Label>
                  <Select
                    value={state.publicChatToolVisibility}
                    onValueChange={(value) =>
                      update("publicChatToolVisibility", value as AgUiToolVisibility)
                    }
                  >
                    <SelectTrigger id="public_chat_tool_visibility">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">
                        {getAgUiToolVisibilityDisplayName("none")}
                      </SelectItem>
                      <SelectItem value="generic">
                        {getAgUiToolVisibilityDisplayName("generic")}
                      </SelectItem>
                      <SelectItem value="narrated">
                        {getAgUiToolVisibilityDisplayName("narrated")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="public_chat_rate_limit">{ui("Rate limit / minute")}</Label>
                  <Input
                    id="public_chat_rate_limit"
                    value={state.publicChatRateLimitPerMinute}
                    onChange={(event) => update("publicChatRateLimitPerMinute", event.target.value)}
                    inputMode="numeric"
                    placeholder={ui("No per-channel cap")}
                  />
                </div>
              </FieldGrid>
              {state.publicChatToolVisibility === "generic" && (
                <div className="space-y-2">
                  <Label htmlFor="public_chat_generic_tool_text">
                    {ui("Generic activity text")}
                  </Label>
                  <Input
                    id="public_chat_generic_tool_text"
                    value={state.publicChatGenericToolText}
                    onChange={(event) => update("publicChatGenericToolText", event.target.value)}
                    maxLength={120}
                  />
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor="public_chat_expiration">{ui("Session expiration (hours)")}</Label>
                <Input
                  id="public_chat_expiration"
                  value={String(state.publicChatExpirationHours)}
                  onChange={(event) => {
                    const parsed = Number.parseFloat(event.target.value);
                    update("publicChatExpirationHours", Number.isFinite(parsed) ? parsed : 0);
                  }}
                  inputMode="numeric"
                  placeholder="6"
                />
                <p className="text-xs text-muted-foreground">
                  {ui("0 = never expire a visitor session.")}
                </p>
              </div>
            </FoldedSettings>
          </FormSection>
        </>
      )}
      {state.kind === "work_chat" && (
        <FormSection
          title="Chat site"
          hint="How conversations are organized, and the limits that apply."
        >
          <WorkChatFields
            mode={state.chatInterface}
            onModeChange={(mode) => update("chatInterface", mode)}
            value={state.api}
            onChange={(api) => update("api", api)}
          />
        </FormSection>
      )}
      {state.kind === "api" && (
        <FormSection
          title="Calls"
          hint="What callers can see and change. Agent keys are made on the channel page after you create it."
        >
          <ApiFields value={state.api} onChange={(api) => update("api", api)} />
        </FormSection>
      )}
      {state.kind === "voice" && (
        <FormSection title="Voice" hint="The voice and how it handles being interrupted.">
          <VoiceFields value={state.voice} onChange={(voice) => update("voice", voice)} />
        </FormSection>
      )}
      {state.kind === "slack" && (
        <>
          <FormSection
            title="Install"
            hint="The agent gets its own Slack app in the workspace you choose."
          >
            <div className="space-y-4">
              {channel && <SlackConnectionStatus channel={channel} />}
              {slackLive && (
                <div className="space-y-3 border p-4">
                  <div className="space-y-1">
                    {!channel && <p className="text-sm font-medium">{ui("Installed in Slack")}</p>}
                    <p className="text-xs text-muted-foreground">
                      {ui("This agent has its own Slack app in workspace")} {state.slackTeamId}
                      {ui(". Mention it in a channel")}
                      {state.slackAgentSurfaceEnabled
                        ? ui(", or open it from Slack’s Agents menu.")
                        : "."}
                    </p>
                  </div>
                  <a
                    className={buttonVariants({ variant: "outline", size: "sm" })}
                    href={slackAppUrl(state.slackAppId, state.slackTeamId)}
                    target="_blank"
                    rel="noreferrer"
                  >
                    <Slack className="size-4" />
                    {ui("Open in Slack")}
                  </a>
                </div>
              )}
              {!slackLive &&
                !state.slackCredentialsConfigured &&
                slackInstallCapability?.supported && (
                  <SlackWorkspaceChoice
                    capability={slackInstallCapability}
                    selected={state.slackInstallTeamId}
                    onSelect={selectSlackInstallTeam}
                    onChanged={onSlackCapabilityChanged}
                  />
                )}
              {!slackLive &&
                mode === "edit" &&
                channelId &&
                slackInstallAvailable &&
                !state.slackCredentialsConfigured &&
                !slackInstall.unavailable && (
                  <div className="space-y-3 border p-4">
                    <div className="space-y-1">
                      <p className="text-sm font-medium">{ui("Add to Slack")}</p>
                      <p className="text-xs text-muted-foreground">
                        {ui(
                          "Creates this agent's own Slack app in the workspace above and opens Slack to approve it. The signing secret, bot token and workspace ID are filled in for you.",
                        )}
                      </p>
                    </div>
                    <Button
                      type="button"
                      onClick={slackInstall.begin}
                      disabled={
                        slackInstall.pending || !state.slackInstallTeamId || slackInstallDisabled
                      }
                    >
                      <Slack className="size-4" />
                      {slackInstall.pending ? ui("Opening Slack…") : ui("Add to Slack")}
                    </Button>
                    {slackInstallDisabled && (
                      <p className="text-xs text-muted-foreground">
                        {ui("Save changes before opening Slack.")}
                      </p>
                    )}
                    {slackInstall.error && (
                      <SlackInstallError error={slackInstall.error} onRetry={slackInstall.begin} />
                    )}
                  </div>
                )}
              {!slackLive && mode === "new" && slackInstallAvailable && (
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "Save the channel to add it to Slack in one click. If you already have a Slack app, open Configure manually below.",
                  )}
                </p>
              )}
            </div>
          </FormSection>
          <FormSection
            title="Conversations"
            hint="When the agent answers and how conversations map to sessions."
          >
            <SlackConversationSettings
              idPrefix={slackFormId}
              sessionStrategy={state.slackSessionStrategy}
              responsePolicy={state.slackResponsePolicy}
              onResponsePolicyChange={(value) => update("slackResponsePolicy", value)}
              onSessionStrategyChange={(value) => update("slackSessionStrategy", value)}
            />

            <SlackAgentPaneSettings
              idPrefix={slackFormId}
              enabled={state.slackAgentSurfaceEnabled}
              configured={state.slackCredentialsConfigured}
              onChange={(checked) => update("slackAgentSurfaceEnabled", checked)}
            />
          </FormSection>
          <FormSection title="Manual setup" hint="Only needed for a Slack app you made yourself.">
            <div>
              <Collapsible
                open={manualSlackOpen}
                onOpenChange={setManualSlackOpen}
                className="border-t pt-4"
              >
                <CollapsibleTrigger asChild>
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 text-left text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    aria-describedby={`${slackFormId}_manual_description`}
                  >
                    <ChevronDown
                      className={cn(
                        "size-4 shrink-0 text-muted-foreground transition-transform",
                        !manualSlackOpen && "-rotate-90",
                      )}
                    />
                    <span>{ui("Configure manually")}</span>
                  </button>
                </CollapsibleTrigger>
                <p
                  id={`${slackFormId}_manual_description`}
                  className="mt-1 pl-6 text-xs leading-relaxed text-muted-foreground"
                >
                  {slackLive || state.slackCredentialsConfigured
                    ? ui("View or update the credentials for this agent’s Slack app.")
                    : slackInstallAvailable
                      ? ui(
                          "Use credentials from an existing Slack app. Add to Slack fills these in for you.",
                        )
                      : ui(
                          "Use credentials from your Slack app to connect this channel. You can save now and finish setup later.",
                        )}
                </p>
                <CollapsibleContent className="space-y-4 pt-4">
                  {manualSlackOpen && channel && <SlackManualSetup channel={channel} />}
                  <p className="text-xs text-muted-foreground">
                    {ui(
                      "These fields are optional when saving. Slack requests are rejected until a signing secret is set; a bot token is needed to send replies.",
                    )}
                    {mode === "edit" && " Leave secret fields blank to keep the saved credentials."}
                  </p>
                  <FieldGrid>
                    <div className="space-y-2">
                      <Label htmlFor={`${slackFormId}_signing_secret`}>
                        {ui("Signing secret")}
                      </Label>
                      <Input
                        id={`${slackFormId}_signing_secret`}
                        type="password"
                        value={state.slackSigningSecret}
                        onChange={(event) => update("slackSigningSecret", event.target.value)}
                        placeholder={
                          mode === "edit"
                            ? ui("Leave blank to keep existing secret")
                            : ui("Slack signing secret")
                        }
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor={`${slackFormId}_bot_token`}>{ui("Bot token")}</Label>
                      <Input
                        id={`${slackFormId}_bot_token`}
                        type="password"
                        value={state.slackBotToken}
                        onChange={(event) => update("slackBotToken", event.target.value)}
                        placeholder={
                          mode === "edit"
                            ? ui("Leave blank to keep existing token")
                            : ui("xoxb-...")
                        }
                      />
                    </div>
                  </FieldGrid>
                  <FieldGrid>
                    <div className="space-y-2">
                      <Label htmlFor={`${slackFormId}_team_id`}>{ui("Workspace ID")}</Label>
                      <Input
                        id={`${slackFormId}_team_id`}
                        value={state.slackTeamId}
                        onChange={(event) => update("slackTeamId", event.target.value)}
                        placeholder={ui("T0123456789")}
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor={`${slackFormId}_channel_id`}>{ui("Channel ID")}</Label>
                      <Input
                        id={`${slackFormId}_channel_id`}
                        aria-describedby={`${slackFormId}_channel_id_description`}
                        value={state.slackChannelId}
                        onChange={(event) => update("slackChannelId", event.target.value)}
                        placeholder={ui("C0123456789")}
                      />
                      <p
                        id={`${slackFormId}_channel_id_description`}
                        className="text-xs leading-relaxed text-muted-foreground"
                      >
                        {ui(
                          "Limit this integration to one Slack channel. Leave blank to accept any Slack channel in the workspace.",
                        )}
                      </p>
                    </div>
                  </FieldGrid>
                </CollapsibleContent>
              </Collapsible>
            </div>
          </FormSection>
        </>
      )}
    </div>
  );
}

/// "20 messages a minute · ends after 6 hours idle", the folded Limits line.
function limitsSummary(
  ratePerMinute: string,
  expirationHours: number,
  ui: (message: string, values?: Record<string, unknown>) => string,
): string {
  const rate = Number.parseInt(ratePerMinute, 10);
  const rateText =
    Number.isFinite(rate) && rate > 0
      ? ui("{value0} messages a minute", { value0: rate })
      : ui("No message limit");
  const idleText =
    expirationHours > 0
      ? ui("conversations end after {value0} hours idle", { value0: expirationHours })
      : ui("conversations never expire");
  return `${rateText} · ${idleText}`;
}
