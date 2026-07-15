"use client";

import {
  CalendarClock,
  Globe,
  Hash,
  MessageSquareText,
  Monitor,
  RefreshCw,
  Webhook,
} from "lucide-react";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import { Button } from "@/components/ui/button";
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
import { Textarea } from "@/components/ui/textarea";
import { CronInput, CronLabel, isSupportedCronExpression } from "@/components/apps/cron-label";
import {
  DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
  DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS,
} from "@/lib/api/types";
import type {
  AgUiChannelConfig,
  AgUiToolVisibility,
  AppEndpointAuthConfig,
  App,
  AppChannel,
  ChannelType,
  FcpChannelConfig,
  InvocationSessionMode,
  PublicChatChannelConfig,
  ScheduleChannelConfig,
  SessionStrategy,
  SlackChannelConfig,
  SlackReplyMode,
  WebhookChannelConfig,
} from "@/lib/api/types";
import {
  getAgUiToolVisibilityDisplayName,
  getChannelTypeDisplayName,
  getInvocationSessionModeDisplayName,
  getSessionStrategyDisplayName,
  getSlackReplyModeDisplayName,
} from "@/lib/app-channels";
import { generateChannelToken } from "@/lib/channel-tokens";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { cn } from "@/lib/utils";

export const CHANNEL_FORM_KINDS: ChannelType[] = [
  "schedule",
  "webhook",
  "ag_ui",
  "public_chat",
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
  slackSessionStrategy: SessionStrategy;
  slackReplyMode: SlackReplyMode;
  scheduleCronExpression: string;
  scheduleTimezone: string;
  invocationSessionMode: InvocationSessionMode;
  channelMessage: string;
  webhookToken: string;
  agUiToken: string;
  agUiAnonymous: boolean;
  agUiAuth?: AppEndpointAuthConfig;
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
};

function secretValue(value?: string, configured?: boolean): string {
  if (value) return value;
  return configured ? "" : "";
}

export function getDefaultChannelFormState(
  kind: ChannelType,
  channel?: AppChannel,
): ChannelFormState {
  const base: ChannelFormState = {
    kind,
    enabled: channel?.enabled ?? true,
    slackSigningSecret: "",
    slackBotToken: "",
    slackTeamId: "",
    slackChannelId: "",
    slackSessionStrategy: "per_thread",
    slackReplyMode: "all_messages",
    scheduleCronExpression: "0 0 * * * * *",
    scheduleTimezone: "UTC",
    invocationSessionMode: "shared_session",
    channelMessage: "",
    webhookToken: "",
    agUiToken: kind === "ag_ui" && !channel ? generateChannelToken() : "",
    agUiAnonymous: true,
    agUiAuth: undefined,
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
    return {
      ...base,
      kind: "ag_ui",
      agUiToken: secretValue(config.token, config.token_configured),
      agUiAnonymous: config.anonymous ?? true,
      agUiAuth: config.auth,
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
      publicChatGoogleEnabled: config.auth?.provider?.type === "google_oidc",
      publicChatGoogleClientId:
        config.auth?.provider?.type === "google_oidc" ? config.auth.provider.client_id : "",
      publicChatGoogleAllowedDomains:
        config.auth?.provider?.type === "google_oidc"
          ? (config.auth.provider.allowed_domains ?? []).join(", ")
          : "",
    };
  }
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelConfig;
    return {
      ...base,
      kind: "slack",
      slackSigningSecret: secretValue(config.signing_secret, config.signing_secret_configured),
      slackBotToken: secretValue(config.bot_token, config.bot_token_configured),
      slackTeamId: config.team_id || "",
      slackChannelId: config.channel_id || "",
      slackSessionStrategy: config.session_strategy || "per_thread",
      slackReplyMode: config.reply_mode || "all_messages",
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
      return {
        anonymous: state.agUiAnonymous,
        ...(state.agUiAuth ? { auth: state.agUiAuth } : {}),
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
    case "slack":
      return {
        ...(state.slackSigningSecret.trim()
          ? { signing_secret: state.slackSigningSecret.trim() }
          : {}),
        ...(state.slackBotToken.trim() ? { bot_token: state.slackBotToken.trim() } : {}),
        ...(state.slackTeamId.trim() ? { team_id: state.slackTeamId.trim() } : {}),
        ...(state.slackChannelId.trim() ? { channel_id: state.slackChannelId.trim() } : {}),
        session_strategy: state.slackSessionStrategy,
        reply_mode: state.slackReplyMode,
      };
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
    // Turning anonymous access off requires a sign-in provider; otherwise the
    // server forbids every request and the chat is unreachable. (A shared token
    // only gates the anonymous path, so it cannot substitute here.)
    if (!state.publicChatAnonymous && !state.publicChatGoogleEnabled) {
      return false;
    }
    return true;
  }
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
      return "Run on a cron-driven cadence in any timezone. App-level automation, not in-session.";
    case "webhook":
      return "Authenticated HTTP endpoint. Bearer token or Everruns webhook token header.";
    case "ag_ui":
      return "Public client surface. Tool names, args, and results are never sent to AG-UI clients.";
    case "public_chat":
      return "Hosted, branded chat website for this one agent. Anonymous or sign-in; optional Turnstile.";
    case "fcp":
      return "Free Communication Protocol. Text-in / text-out HTTP endpoint with a Markdown handshake.";
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
  const kinds = CHANNEL_FORM_KINDS.filter((kind) => kind !== "public_chat" || publicChatEnabled);
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
}: {
  state: ChannelFormState;
  onChange: (state: ChannelFormState) => void;
  mode: "new" | "edit";
  section?: ChannelFormSection;
}) {
  const update = <K extends keyof ChannelFormState>(key: K, value: ChannelFormState[K]) =>
    onChange({ ...state, [key]: value });

  if (section === "runs") {
    return (
      <div className="border border-dashed p-4 text-sm text-muted-foreground">
        Run history will appear here when the app run aggregation endpoint is available.
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {mode === "edit" && section === "all" && (
        <div className="flex items-center justify-between border p-3">
          <div>
            <p className="text-sm font-medium">Enabled</p>
            <p className="text-xs text-muted-foreground">
              Disabled channels stay configured but do not invoke the app.
            </p>
          </div>
          <Switch
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
                on the endpoint URL) stays public so clients can discover how to authenticate.
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
              placeholder="Leave blank to auto-generate from the app name and description."
              rows={4}
            />
            <p className="text-xs text-muted-foreground">
              Returned verbatim for GET on the endpoint URL. Use plain Markdown.
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
              How long the endpoint waits for the agent before returning 504. Must be 1-600s.
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
                  placeholder="Falls back to the app name"
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

          <div className="space-y-3">
            <div className="flex items-center justify-between border p-3">
              <div>
                <p className="text-sm font-medium">Google sign-in</p>
                <p className="text-xs text-muted-foreground">
                  Require visitors to sign in with Google. When on, every request must present a
                  valid Google account; signed-in visitors skip the Turnstile challenge.
                </p>
              </div>
              <Switch
                checked={state.publicChatGoogleEnabled}
                onCheckedChange={(checked) => update("publicChatGoogleEnabled", checked)}
              />
            </div>
            {state.publicChatGoogleEnabled && (
              <FieldGrid>
                <div className="space-y-2">
                  <Label htmlFor="public_chat_google_client_id">Google OAuth client ID</Label>
                  <Input
                    id="public_chat_google_client_id"
                    value={state.publicChatGoogleClientId}
                    onChange={(event) => update("publicChatGoogleClientId", event.target.value)}
                    className="font-mono"
                    placeholder="1234-abc.apps.googleusercontent.com"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="public_chat_google_domains">Allowed domains (optional)</Label>
                  <Input
                    id="public_chat_google_domains"
                    value={state.publicChatGoogleAllowedDomains}
                    onChange={(event) =>
                      update("publicChatGoogleAllowedDomains", event.target.value)
                    }
                    placeholder="example.com, partner.com"
                  />
                  <p className="text-xs text-muted-foreground">
                    Comma-separated. Leave blank to allow any Google account.
                  </p>
                </div>
              </FieldGrid>
            )}
          </div>

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

      {state.kind === "slack" && (section === "all" || section === "invocation") && (
        <div className="space-y-4">
          <FieldGrid>
            <div className="space-y-2">
              <Label htmlFor="slack_signing_secret">Signing secret</Label>
              <Input
                id="slack_signing_secret"
                type="password"
                value={state.slackSigningSecret}
                onChange={(event) => update("slackSigningSecret", event.target.value)}
                placeholder={
                  mode === "edit" ? "Leave blank to keep existing secret" : "Slack signing secret"
                }
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="slack_bot_token">Bot token</Label>
              <Input
                id="slack_bot_token"
                type="password"
                value={state.slackBotToken}
                onChange={(event) => update("slackBotToken", event.target.value)}
                placeholder={mode === "edit" ? "Leave blank to keep existing token" : "xoxb-..."}
              />
            </div>
          </FieldGrid>
          <FieldGrid>
            <div className="space-y-2">
              <Label htmlFor="slack_team_id">Workspace ID</Label>
              <Input
                id="slack_team_id"
                value={state.slackTeamId}
                onChange={(event) => update("slackTeamId", event.target.value)}
                placeholder="T0123456789"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="slack_channel_id">Channel ID</Label>
              <Input
                id="slack_channel_id"
                value={state.slackChannelId}
                onChange={(event) => update("slackChannelId", event.target.value)}
                placeholder="C0123456789"
              />
            </div>
          </FieldGrid>
        </div>
      )}

      {state.kind === "slack" && (section === "all" || section === "session") && (
        <FieldGrid>
          <div className="space-y-2">
            <Label htmlFor="slack_session_strategy">Session strategy</Label>
            <Select
              value={state.slackSessionStrategy}
              onValueChange={(value) => update("slackSessionStrategy", value as SessionStrategy)}
            >
              <SelectTrigger id="slack_session_strategy">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="per_thread">
                  {getSessionStrategyDisplayName("per_thread")}
                </SelectItem>
                <SelectItem value="per_channel">
                  {getSessionStrategyDisplayName("per_channel")}
                </SelectItem>
                <SelectItem value="per_user">
                  {getSessionStrategyDisplayName("per_user")}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="slack_reply_mode">Reply mode</Label>
            <Select
              value={state.slackReplyMode}
              onValueChange={(value) => update("slackReplyMode", value as SlackReplyMode)}
            >
              <SelectTrigger id="slack_reply_mode">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all_messages">
                  {getSlackReplyModeDisplayName("all_messages")}
                </SelectItem>
                <SelectItem value="report_progress_only">
                  {getSlackReplyModeDisplayName("report_progress_only")}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
        </FieldGrid>
      )}
    </div>
  );
}

export function ChannelFormSummary({ app, state }: { app?: App; state: ChannelFormState }) {
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
              Publish {app?.name ?? "the app"} before external clients can invoke this endpoint.
            </p>
          </div>
        )}
        {state.kind === "slack" && (
          <div>
            <p className="text-xs font-medium uppercase text-muted-foreground">Slack</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Reuse the existing manifest flow after saving credentials.
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
