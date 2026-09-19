"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { EndpointUsePanel } from "@/components/agents/endpoint-use-panel";
import { A2aSetupGuidance } from "@/components/agents/integrations/a2a-setup-guidance";
import { AgUiSetupGuidance } from "@/components/agents/integrations/ag-ui-setup-guidance";
import { FcpSetupGuidance } from "@/components/agents/integrations/fcp-setup-guidance";
import { SlackSetupGuidance } from "@/components/agents/integrations/slack-setup-guidance";
import { getSlackEndpointManifest, type SlackManifest } from "@/lib/api/agent-endpoints";
import type {
  A2aChannelConfig,
  AgUiChannelConfig,
  AppChannel,
  FcpChannelConfig,
  SlackChannelConfig,
} from "@/lib/api/types";
import { getEndpointLifecyclePresentation } from "@/lib/app-channels";
import { isPublicHttpsUrl } from "@/lib/public-origin";
type SlackEndpointConfig = SlackChannelConfig & {
  agent_surface_enabled?: boolean;
};

function endpointUrl(endpointId: string, suffix: string): string {
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  return `${origin}/api/v1/e/${endpointId}/${suffix}`;
}
export function getSlackManifestRequestUrl(manifestYaml: string): string | null {
  const eventSubscriptions = manifestYaml.match(
    /event_subscriptions:\s*\n\s*request_url:\s*"([^"]+)"/,
  );
  return eventSubscriptions?.[1] ?? null;
}

function SetupHeading() {
  return <p className="text-xs font-medium uppercase text-muted-foreground">Set up</p>;
}

function EndpointSetupGuidance({
  agentName,
  agentDescription,
  channel,
  configureHref,
}: {
  agentName: string;
  agentDescription?: string | null;
  channel: AppChannel;
  configureHref?: string;
}) {
  const router = useRouter();
  const [slackManifest, setSlackManifest] = useState<SlackManifest | null>(null);
  const [slackManifestError, setSlackManifestError] = useState<string | null>(null);
  const lifecycle = getEndpointLifecyclePresentation(channel);
  const configure = configureHref ? () => router.push(configureHref) : undefined;
  const slackConfig =
    channel.channel_type === "slack" ? (channel.channel_config as SlackEndpointConfig) : null;
  const hasSlackConfig =
    !!(slackConfig?.signing_secret_configured || slackConfig?.signing_secret) &&
    !!(slackConfig?.bot_token_configured || slackConfig?.bot_token);
  const shouldLoadSlackManifest =
    channel.channel_type === "slack" && lifecycle.isLive && !hasSlackConfig;

  useEffect(() => {
    if (!shouldLoadSlackManifest) return;

    let ignore = false;
    getSlackEndpointManifest(channel.id)
      .then((manifest) => {
        if (!ignore) setSlackManifest(manifest);
      })
      .catch(() => {
        if (!ignore) {
          setSlackManifestError("Could not load the Slack app manifest. Try again.");
        }
      });

    return () => {
      ignore = true;
    };
  }, [channel.id, shouldLoadSlackManifest]);

  if (channel.channel_type === "slack") {
    const config = slackConfig as SlackEndpointConfig;
    const manifestRequestUrl = getSlackManifestRequestUrl(slackManifest?.manifest_yaml ?? "");
    const canCreateSlackApp = isPublicHttpsUrl(manifestRequestUrl);

    return (
      <div className="space-y-3">
        <SetupHeading />
        <SlackSetupGuidance
          hasSlackConfig={hasSlackConfig}
          isPublished={lifecycle.isLive}
          webhookVerified={!!config.webhook_verified_at}
          firstMessageReceived={!!config.first_message_received_at}
          manifestRequestUrl={manifestRequestUrl}
          manifestLoading={shouldLoadSlackManifest && !slackManifest && !slackManifestError}
          canCreateSlackApp={canCreateSlackApp}
          agentSurfaceEnabled={config.agent_surface_enabled ?? false}
          onCreateSlackApp={() => {
            if (slackManifest && canCreateSlackApp) {
              window.open(slackManifest.create_url, "_blank", "noopener,noreferrer");
            }
          }}
          onConfigure={configure}
        />
        {slackManifestError && <p className="text-xs text-destructive">{slackManifestError}</p>}
      </div>
    );
  }

  if (channel.channel_type === "a2a") {
    const config = channel.channel_config as A2aChannelConfig;
    const url = endpointUrl(channel.id, "a2a");
    return (
      <div className="space-y-3">
        <SetupHeading />
        <A2aSetupGuidance
          endpointUrl={url}
          agentCardUrl={`${url}/.well-known/agent-card.json`}
          apiKeyPrefix={config.api_key_prefix}
          sessionMode={config.session_mode ?? "shared_session"}
          message={config.message}
          agentName={agentName}
          agentCardName={config.agent_card_name}
          agentCardDescription={config.agent_card_description ?? agentDescription ?? undefined}
          isPublished={lifecycle.isLive}
          channelEnabled={channel.enabled}
          onConfigure={configure}
        />
      </div>
    );
  }

  if (channel.channel_type === "ag_ui") {
    const config = channel.channel_config as AgUiChannelConfig;
    return (
      <div className="space-y-3">
        <SetupHeading />
        <AgUiSetupGuidance
          endpointUrl={endpointUrl(channel.id, "ag-ui")}
          imageUploadUrl={endpointUrl(channel.id, "ag-ui/images")}
          isPublished={lifecycle.isLive}
          anonymousEnabled={config.anonymous ?? false}
          sessionExpirationSeconds={config.session_expiration_seconds ?? 0}
          rateLimitPerMinute={config.rate_limit_per_minute}
          tokenConfigured={config.token_configured}
          toolVisibility={config.tool_visibility}
          genericToolText={config.generic_tool_text}
          onConfigure={configure}
        />
      </div>
    );
  }

  if (channel.channel_type === "fcp") {
    const config = channel.channel_config as FcpChannelConfig;
    return (
      <div className="space-y-3">
        <SetupHeading />
        <FcpSetupGuidance
          endpointUrl={endpointUrl(channel.id, "fcp")}
          isPublished={lifecycle.isLive}
          anonymousEnabled={config.anonymous ?? false}
          sessionExpirationSeconds={config.session_expiration_seconds ?? 0}
          rateLimitPerMinute={config.rate_limit_per_minute}
          responseTimeoutSeconds={config.response_timeout_seconds}
          tokenConfigured={config.token_configured}
          hasCustomHandshake={!!config.handshake}
          onConfigure={configure}
        />
      </div>
    );
  }

  return null;
}

export function EndpointDetailsPanel({
  agentName,
  agentDescription,
  channel,
  configureHref,
}: {
  agentName: string;
  agentDescription?: string | null;
  channel: AppChannel;
  configureHref?: string;
}) {
  return (
    <div className="space-y-6">
      <EndpointSetupGuidance
        agentName={agentName}
        agentDescription={agentDescription}
        channel={channel}
        configureHref={configureHref}
      />
      <EndpointUsePanel channel={channel} />
    </div>
  );
}
