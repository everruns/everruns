"use client";

import { useCallback, useId, useState } from "react";
import {
  CalendarClock,
  ChevronDown,
  Globe,
  Hash,
  MessageSquareText,
  Mic,
  Monitor,
  RefreshCw,
  Webhook,
} from "lucide-react";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import { Button, buttonVariants } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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
import { CronInput, CronLabel, isSupportedCronExpression } from "@/components/apps/cron-label";
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
  SlackReplyMode,
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
  CHANNEL_DISABLE_HINT,
  getAgUiToolVisibilityDisplayName,
  getChannelTypeDisplayName,
  getInvocationSessionModeDisplayName,
} from "@/lib/channel-display";
import { generateChannelToken } from "@/lib/channel-tokens";
import { slackAppUrl } from "@/components/slack/slack-workspaces";
import { SlackWorkspaceChoice, useSlackInstall } from "./slack-install";
import type { SlackInstallCapability } from "@/lib/api/agent-channels";
import { SlackInstallError } from "@/components/slack/slack-install-error";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { cn } from "@/lib/utils";

export const CHANNEL_FORM_KINDS: ChannelType[] = [
  "schedule",
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

export type ChannelFormSection = "all" | "schedule" | "invocation" | "session" | "runs";

export type ChannelFormState = {
  kind: ChannelType;
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
  slackReplyMode: SlackReplyMode;
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
    slackReplyMode: "all_messages",
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
      slackReplyMode: config.reply_mode || "all_messages",
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
  if (state.kind === "slack") return true;
  return false;
}

function channelIcon(kind: ChannelType) {
  switch (kind) {
    case "schedule":
      return CalendarClock;
    case "webhook":
      return Webhook;
    case "ag_ui":
      return Monitor;
    case "public_chat":
      return Globe;
    case "voice":
      return Mic;
    case "fcp":
      return MessageSquareText;
    case "slack":
      return Slack;
    default:
      return Hash;
  }
}

function channelDescription(kind: ChannelType): string {
  switch (kind) {
    case "schedule":
      return "Run this agent on a cron-driven cadence in any timezone.";
    case "webhook":
      return "Authenticated HTTP channel. Bearer token or Everruns webhook token header.";
    case "ag_ui":
      return "Public client surface. Tool names, args, and results are never sent to AG-UI clients.";
    case "public_chat":
      return "Hosted, branded chat website for this one agent. Anonymous or sign-in; optional Turnstile.";
    case "voice":
      return "Talk to this agent by voice. A speech model listens and speaks; the agent answers.";
    case "fcp":
      return "Free Communication Protocol. Text-in / text-out HTTP channel with a Markdown handshake.";
    case "slack":
      return "React to messages in Slack. Per-thread, channel, or user session strategies.";
    default:
      return "Channel";
  }
}

export function ChannelTypePicker({
  value,
  onChange,
}: {
  value: ChannelType;
  onChange: (value: ChannelType) => void;
}) {
  const publicChatEnabled = useFeatureFlag("public_chat");
  const voiceEnabled = useFeatureFlag("voice");
  const kinds = CHANNEL_FORM_KINDS.filter(
    (kind) =>
      kind !== "schedule" &&
      (kind !== "public_chat" || publicChatEnabled) &&
      (kind !== "voice" || voiceEnabled),
  );
  return (
    <div className="grid gap-3 md:grid-cols-2">
      {kinds.map((kind) => {
        const Icon = channelIcon(kind);
        const selected = value === kind;
        return (
          <button
            key={kind}
            type="button"
            onClick={() => onChange(kind)}
            className={cn(
              "border p-4 text-left transition-colors hover:bg-muted/40",
              selected ? "border-primary bg-muted" : "border-border",
            )}
          >
            <div className="flex items-start justify-between gap-3">
              <div className="flex gap-3">
                <span className="flex size-8 items-center justify-center border bg-background">
                  <Icon className="size-4" />
                </span>
                <div>
                  <p className="font-medium">{getChannelTypeDisplayName(kind)}</p>
                  <p className="mt-1 text-sm text-muted-foreground">{channelDescription(kind)}</p>
                </div>
              </div>
              {selected && <span className="text-sm font-medium">Selected</span>}
            </div>
          </button>
        );
      })}
    </div>
  );
}

function FieldGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid gap-4 md:grid-cols-2">{children}</div>;
}

export function ChannelForm({
  state,
  onChange,
  mode,
  section = "all",
  channelId,
  channel,
  slackInstallDisabled = false,
  slackInstallCapability,
  onSlackCapabilityChanged,
}: {
  state: ChannelFormState;
  onChange: (state: ChannelFormState) => void;
  mode: "new" | "edit";
  section?: ChannelFormSection;
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

  if (section === "runs") {
    return (
      <div className="border border-dashed p-4 text-sm text-muted-foreground">
        Run history will appear here when channel run aggregation is available.
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {mode === "edit" && section === "all" && (
        <div className="flex items-center justify-between border p-3">
          <div>
            <p className="text-sm font-medium">Enabled</p>
            <p className="text-xs text-muted-foreground">{CHANNEL_DISABLE_HINT}</p>
          </div>
          <Switch
            aria-label="Enabled"
            checked={state.enabled}
            onCheckedChange={(checked) => update("enabled", checked)}
          />
        </div>
      )}

      {state.kind === "schedule" && (section === "all" || section === "schedule") && (
        <CronInput
          value={state.scheduleCronExpression}
          timezone={state.scheduleTimezone}
          onChange={(value) => update("scheduleCronExpression", value)}
          onTimezoneChange={(value) => update("scheduleTimezone", value)}
        />
      )}

      {(state.kind === "schedule" || state.kind === "webhook") &&
        (section === "all" || section === "invocation") && (
          <div className="space-y-2">
            <Label htmlFor="channel_message">Invocation message</Label>
            <Textarea
              id="channel_message"
              value={state.channelMessage}
              onChange={(event) => update("channelMessage", event.target.value)}
              placeholder={
                state.kind === "schedule"
                  ? "Run {{app.name}} now."
                  : "Process webhook payload for {{payload.repo.name}}."
              }
            />
          </div>
        )}

      {(state.kind === "schedule" || state.kind === "webhook") &&
        (section === "all" || section === "session") && (
          <div className="space-y-2">
            <Label htmlFor="session_mode">Session mode</Label>
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
        )}

      {state.kind === "webhook" && (section === "all" || section === "invocation") && (
        <div className="space-y-2">
          <Label htmlFor="webhook_token">Webhook token</Label>
          <Input
            id="webhook_token"
            type="password"
            value={state.webhookToken}
            onChange={(event) => update("webhookToken", event.target.value)}
            placeholder={mode === "edit" ? "Leave blank to keep existing token" : "shared-secret"}
          />
          <p className="text-xs text-muted-foreground">
            Send as Authorization: Bearer &lt;token&gt; or X-Everruns-Webhook-Token.
          </p>
        </div>
      )}

      {state.kind === "ag_ui" && (section === "all" || section === "invocation") && (
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="ag_ui_token">Bearer token</Label>
            <div className="flex gap-2">
              <Input
                id="ag_ui_token"
                value={state.agUiToken}
                onChange={(event) => update("agUiToken", event.target.value)}
                className="font-mono"
                placeholder={
                  mode === "edit" ? "Leave blank to keep existing token" : "Generated bearer token"
                }
              />
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={() => update("agUiToken", generateChannelToken())}
                aria-label="Regenerate AG-UI token"
              >
                <RefreshCw className="size-4" />
              </Button>
            </div>
          </div>
          <AgentIdSignInFields
            idPrefix="ag_ui"
            description="Accept AgentID tokens from AI agents. When on, every request must present a valid AgentID token for your client and the bearer token above is not used."
            enabled={state.agUiAgentIdEnabled}
            clientId={state.agUiAgentIdClientId}
            onEnabledChange={(checked) => update("agUiAgentIdEnabled", checked)}
            onClientIdChange={(value) => update("agUiAgentIdClientId", value)}
          />
          <FieldGrid>
            <div className="space-y-2">
              <Label htmlFor="ag_ui_tool_visibility">Tool visibility</Label>
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
            <div className="space-y-2">
              <Label htmlFor="ag_ui_rate_limit">Rate limit / minute</Label>
              <Input
                id="ag_ui_rate_limit"
                value={state.agUiRateLimitPerMinute}
                onChange={(event) => update("agUiRateLimitPerMinute", event.target.value)}
                inputMode="numeric"
                placeholder="No per-channel cap"
              />
            </div>
          </FieldGrid>
          {state.agUiToolVisibility === "generic" && (
            <div className="space-y-2">
              <Label htmlFor="ag_ui_generic_tool_text">Generic activity text</Label>
              <Input
                id="ag_ui_generic_tool_text"
                value={state.agUiGenericToolText}
                onChange={(event) => update("agUiGenericToolText", event.target.value)}
                maxLength={120}
              />
              <p className="text-xs text-muted-foreground">
                Tool names, arguments, and results are never sent to public clients.
              </p>
            </div>
          )}
        </div>
      )}

      {state.kind === "fcp" && (section === "all" || section === "invocation") && (
        <div className="space-y-4">
          <div className="flex items-center justify-between border p-3">
            <div>
              <p className="text-sm font-medium">Anonymous access</p>
              <p className="text-xs text-muted-foreground">
                When off, every request must present the bearer token below. The FCP handshake (GET
                on the channel URL) stays public so clients can discover how to authenticate.
              </p>
            </div>
            <Switch
              checked={state.fcpAnonymous}
              onCheckedChange={(checked) => update("fcpAnonymous", checked)}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="fcp_token">Bearer token</Label>
            <div className="flex gap-2">
              <Input
                id="fcp_token"
                value={state.fcpToken}
                onChange={(event) => update("fcpToken", event.target.value)}
                className="font-mono"
                placeholder={
                  mode === "edit" ? "Leave blank to keep existing token" : "Generated bearer token"
                }
              />
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={() => update("fcpToken", generateChannelToken())}
                aria-label="Regenerate FCP token"
              >
                <RefreshCw className="size-4" />
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              Verified with constant-time comparison; sent as Authorization: Bearer &lt;token&gt; or
              X-Everruns-FCP-Token. Never echoed in responses.
            </p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="fcp_handshake">Custom handshake (Markdown)</Label>
            <Textarea
              id="fcp_handshake"
              value={state.fcpHandshake}
              onChange={(event) => update("fcpHandshake", event.target.value)}
              placeholder="Leave blank to auto-generate from the agent name and description."
              rows={4}
            />
            <p className="text-xs text-muted-foreground">
              Returned verbatim for GET on the channel URL. Use plain Markdown.
            </p>
          </div>
          <FieldGrid>
            <div className="space-y-2">
              <Label htmlFor="fcp_expiration">Session expiration (hours)</Label>
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
                0 = never expire the fcp_session cookie.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="fcp_rate_limit">Rate limit / minute</Label>
              <Input
                id="fcp_rate_limit"
                value={state.fcpRateLimitPerMinute}
                onChange={(event) => update("fcpRateLimitPerMinute", event.target.value)}
                inputMode="numeric"
                placeholder="No per-channel cap"
              />
              <p className="text-xs text-muted-foreground">
                FCP-only limiter namespace; cannot share buckets with other channels.
              </p>
            </div>
          </FieldGrid>
          <div className="space-y-2">
            <Label htmlFor="fcp_response_timeout">Response timeout (seconds)</Label>
            <Input
              id="fcp_response_timeout"
              value={state.fcpResponseTimeoutSeconds}
              onChange={(event) => update("fcpResponseTimeoutSeconds", event.target.value)}
              inputMode="numeric"
              placeholder="120"
            />
            <p className="text-xs text-muted-foreground">
              How long the channel waits for the agent before returning 504. Must be 1-600s.
            </p>
          </div>
        </div>
      )}

      {state.kind === "public_chat" && (section === "all" || section === "invocation") && (
        <div className="space-y-4">
          <div className="flex items-center justify-between border p-3">
            <div>
              <p className="text-sm font-medium">Anonymous access</p>
              <p className="text-xs text-muted-foreground">
                When on, anyone with the link can chat (optionally gated by the shared token below).
                Turn off to require sign-in — you must enable Google sign-in below, otherwise the
                chat rejects every request.
              </p>
            </div>
            <Switch
              checked={state.publicChatAnonymous}
              onCheckedChange={(checked) => update("publicChatAnonymous", checked)}
            />
          </div>

          <div className="space-y-3">
            <p className="text-sm font-medium">Branding</p>
            <FieldGrid>
              <div className="space-y-2">
                <Label htmlFor="public_chat_display_name">Display name</Label>
                <Input
                  id="public_chat_display_name"
                  value={state.publicChatDisplayName}
                  onChange={(event) => update("publicChatDisplayName", event.target.value)}
                  maxLength={120}
                  placeholder="Falls back to the agent name"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="public_chat_primary_color">Primary color</Label>
                <Input
                  id="public_chat_primary_color"
                  value={state.publicChatPrimaryColor}
                  onChange={(event) => update("publicChatPrimaryColor", event.target.value)}
                  placeholder="#0A1636"
                />
              </div>
            </FieldGrid>
            <div className="space-y-2">
              <Label htmlFor="public_chat_logo_url">Logo URL</Label>
              <Input
                id="public_chat_logo_url"
                value={state.publicChatLogoUrl}
                onChange={(event) => update("publicChatLogoUrl", event.target.value)}
                placeholder="https://example.com/logo.png"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="public_chat_welcome">Welcome message</Label>
              <Textarea
                id="public_chat_welcome"
                value={state.publicChatWelcomeMessage}
                onChange={(event) => update("publicChatWelcomeMessage", event.target.value)}
                placeholder="Shown before the visitor's first message."
                rows={2}
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="public_chat_token">Shared token (optional)</Label>
            <div className="flex gap-2">
              <Input
                id="public_chat_token"
                value={state.publicChatToken}
                onChange={(event) => update("publicChatToken", event.target.value)}
                className="font-mono"
                placeholder={
                  mode === "edit" ? "Leave blank to keep existing token" : "Optional bearer token"
                }
              />
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={() => update("publicChatToken", generateChannelToken())}
                aria-label="Regenerate Public Chat token"
              >
                <RefreshCw className="size-4" />
              </Button>
            </div>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between border p-3">
              <div>
                <p className="text-sm font-medium">Bot mitigation (Cloudflare Turnstile)</p>
                <p className="text-xs text-muted-foreground">
                  Challenge anonymous visitors before they can chat. Signed-in visitors are exempt.
                </p>
              </div>
              <Switch
                checked={state.publicChatCaptchaEnabled}
                onCheckedChange={(checked) => update("publicChatCaptchaEnabled", checked)}
              />
            </div>
            <FieldGrid>
              <div className="space-y-2">
                <Label htmlFor="public_chat_turnstile_site_key">Turnstile site key</Label>
                <Input
                  id="public_chat_turnstile_site_key"
                  value={state.publicChatTurnstileSiteKey}
                  onChange={(event) => update("publicChatTurnstileSiteKey", event.target.value)}
                  className="font-mono"
                  placeholder="0x4AAAAAAA..."
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="public_chat_turnstile_secret_key">Turnstile secret key</Label>
                <Input
                  id="public_chat_turnstile_secret_key"
                  type="password"
                  value={state.publicChatTurnstileSecretKey}
                  onChange={(event) => update("publicChatTurnstileSecretKey", event.target.value)}
                  className="font-mono"
                  placeholder={
                    mode === "edit" ? "Leave blank to keep existing secret" : "0x4AAAAAAA..."
                  }
                />
              </div>
            </FieldGrid>
            <p className="text-xs text-muted-foreground">
              The secret key is write-only and never shown after saving.
            </p>
          </div>

          <PublicChatSignInFields
            googleEnabled={state.publicChatGoogleEnabled}
            googleClientId={state.publicChatGoogleClientId}
            googleAllowedDomains={state.publicChatGoogleAllowedDomains}
            agentIdEnabled={state.publicChatAgentIdEnabled}
            agentIdClientId={state.publicChatAgentIdClientId}
            update={update}
          />

          <FieldGrid>
            <div className="space-y-2">
              <Label htmlFor="public_chat_tool_visibility">Tool visibility</Label>
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
            <div className="space-y-2">
              <Label htmlFor="public_chat_rate_limit">Rate limit / minute</Label>
              <Input
                id="public_chat_rate_limit"
                value={state.publicChatRateLimitPerMinute}
                onChange={(event) => update("publicChatRateLimitPerMinute", event.target.value)}
                inputMode="numeric"
                placeholder="No per-channel cap"
              />
            </div>
          </FieldGrid>
          {state.publicChatToolVisibility === "generic" && (
            <div className="space-y-2">
              <Label htmlFor="public_chat_generic_tool_text">Generic activity text</Label>
              <Input
                id="public_chat_generic_tool_text"
                value={state.publicChatGenericToolText}
                onChange={(event) => update("publicChatGenericToolText", event.target.value)}
                maxLength={120}
              />
            </div>
          )}
          <div className="space-y-2">
            <Label htmlFor="public_chat_expiration">Session expiration (hours)</Label>
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
            <p className="text-xs text-muted-foreground">0 = never expire a visitor session.</p>
          </div>
        </div>
      )}

      {state.kind === "voice" && (section === "all" || section === "invocation") && (
        <VoiceFields value={state.voice} onChange={(voice) => update("voice", voice)} />
      )}

      {state.kind === "slack" && (section === "all" || section === "invocation") && (
        <div className="space-y-4">
          {channel && <SlackConnectionStatus channel={channel} />}
          {slackLive && (
            <div className="space-y-3 border p-4">
              <div className="space-y-1">
                {!channel && <p className="text-sm font-medium">Installed in Slack</p>}
                <p className="text-xs text-muted-foreground">
                  This agent has its own Slack app in workspace {state.slackTeamId}. Mention it in a
                  channel
                  {state.slackAgentSurfaceEnabled ? ", or open it from Slack’s Agents menu." : "."}
                </p>
              </div>
              <a
                className={buttonVariants({ variant: "outline", size: "sm" })}
                href={slackAppUrl(state.slackAppId, state.slackTeamId)}
                target="_blank"
                rel="noreferrer"
              >
                <Slack className="size-4" />
                Open in Slack
              </a>
            </div>
          )}
          {!slackLive && !state.slackCredentialsConfigured && slackInstallCapability?.supported && (
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
                  <p className="text-sm font-medium">Add to Slack</p>
                  <p className="text-xs text-muted-foreground">
                    Creates this agent&apos;s own Slack app in the workspace above and opens Slack
                    to approve it. The signing secret, bot token and workspace ID are filled in for
                    you.
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
                  {slackInstall.pending ? "Opening Slack…" : "Add to Slack"}
                </Button>
                {slackInstallDisabled && (
                  <p className="text-xs text-muted-foreground">
                    Save changes before opening Slack.
                  </p>
                )}
                {slackInstall.error && (
                  <SlackInstallError error={slackInstall.error} onRetry={slackInstall.begin} />
                )}
              </div>
            )}
          {!slackLive && mode === "new" && slackInstallAvailable && (
            <p className="text-xs text-muted-foreground">
              Save the channel to add it to Slack in one click. If you already have a Slack app,
              open Configure manually below.
            </p>
          )}
        </div>
      )}

      {state.kind === "slack" && (section === "all" || section === "session") && (
        <SlackConversationSettings
          idPrefix={slackFormId}
          sessionStrategy={state.slackSessionStrategy}
          replyMode={state.slackReplyMode}
          responsePolicy={state.slackResponsePolicy}
          onResponsePolicyChange={(value) => update("slackResponsePolicy", value)}
          onSessionStrategyChange={(value) => update("slackSessionStrategy", value)}
          onReplyModeChange={(value) => update("slackReplyMode", value)}
        />
      )}

      {state.kind === "slack" && (section === "all" || section === "session") && (
        <SlackAgentPaneSettings
          idPrefix={slackFormId}
          enabled={state.slackAgentSurfaceEnabled}
          configured={state.slackCredentialsConfigured}
          onChange={(checked) => update("slackAgentSurfaceEnabled", checked)}
        />
      )}

      {state.kind === "slack" && (section === "all" || section === "invocation") && (
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
                <span>Configure manually</span>
              </button>
            </CollapsibleTrigger>
            <p
              id={`${slackFormId}_manual_description`}
              className="mt-1 pl-6 text-xs leading-relaxed text-muted-foreground"
            >
              {slackLive || state.slackCredentialsConfigured
                ? "View or update the credentials for this agent’s Slack app."
                : slackInstallAvailable
                  ? "Use credentials from an existing Slack app. Add to Slack fills these in for you."
                  : "Use credentials from your Slack app to connect this channel. You can save now and finish setup later."}
            </p>
            <CollapsibleContent className="space-y-4 pt-4">
              {manualSlackOpen && channel && <SlackManualSetup channel={channel} />}
              <p className="text-xs text-muted-foreground">
                These fields are optional when saving. Slack requests are rejected until a signing
                secret is set; a bot token is needed to send replies.
                {mode === "edit" && " Leave secret fields blank to keep the saved credentials."}
              </p>
              <FieldGrid>
                <div className="space-y-2">
                  <Label htmlFor={`${slackFormId}_signing_secret`}>Signing secret</Label>
                  <Input
                    id={`${slackFormId}_signing_secret`}
                    type="password"
                    value={state.slackSigningSecret}
                    onChange={(event) => update("slackSigningSecret", event.target.value)}
                    placeholder={
                      mode === "edit"
                        ? "Leave blank to keep existing secret"
                        : "Slack signing secret"
                    }
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor={`${slackFormId}_bot_token`}>Bot token</Label>
                  <Input
                    id={`${slackFormId}_bot_token`}
                    type="password"
                    value={state.slackBotToken}
                    onChange={(event) => update("slackBotToken", event.target.value)}
                    placeholder={
                      mode === "edit" ? "Leave blank to keep existing token" : "xoxb-..."
                    }
                  />
                </div>
              </FieldGrid>
              <FieldGrid>
                <div className="space-y-2">
                  <Label htmlFor={`${slackFormId}_team_id`}>Workspace ID</Label>
                  <Input
                    id={`${slackFormId}_team_id`}
                    value={state.slackTeamId}
                    onChange={(event) => update("slackTeamId", event.target.value)}
                    placeholder="T0123456789"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor={`${slackFormId}_channel_id`}>Channel ID</Label>
                  <Input
                    id={`${slackFormId}_channel_id`}
                    aria-describedby={`${slackFormId}_channel_id_description`}
                    value={state.slackChannelId}
                    onChange={(event) => update("slackChannelId", event.target.value)}
                    placeholder="C0123456789"
                  />
                  <p
                    id={`${slackFormId}_channel_id_description`}
                    className="text-xs leading-relaxed text-muted-foreground"
                  >
                    Limit this integration to one Slack channel. Leave blank to accept any Slack
                    channel in the workspace.
                  </p>
                </div>
              </FieldGrid>
            </CollapsibleContent>
          </Collapsible>
        </div>
      )}
    </div>
  );
}

export function ChannelFormSummary({ state }: { state: ChannelFormState }) {
  return (
    <Card className="h-fit">
      <CardContent className="space-y-4 py-4">
        <div>
          <p className="text-xs font-medium uppercase text-muted-foreground">Channel</p>
          <p className="mt-1 font-medium">{getChannelTypeDisplayName(state.kind)}</p>
        </div>
        {state.kind === "schedule" && (
          <div>
            <p className="text-xs font-medium uppercase text-muted-foreground">Schedule preview</p>
            <p className="mt-1 text-sm">
              <CronLabel expr={state.scheduleCronExpression} tz={state.scheduleTimezone} />
            </p>
          </div>
        )}
        {(state.kind === "webhook" || state.kind === "ag_ui" || state.kind === "fcp") && (
          <div>
            <p className="text-xs font-medium uppercase text-muted-foreground">Activation</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Publish this channel before external clients can invoke it.
            </p>
          </div>
        )}
        {state.kind === "voice" && (
          <div>
            <p className="text-xs font-medium uppercase text-muted-foreground">Voice</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Needs an OpenAI provider. Call it from the channel, the chat microphone, or the API.
            </p>
          </div>
        )}
        {state.kind === "slack" && (
          <div>
            <p className="text-xs font-medium uppercase text-muted-foreground">Slack</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Gets its own Slack app, with its own name in Slack&apos;s Agents menu, in the
              workspace you choose.
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
