"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { EndpointUsePanel } from "@/components/agents/endpoint-use-panel";
import { A2aSetupGuidance } from "@/components/agents/integrations/a2a-setup-guidance";
import { AgUiSetupGuidance } from "@/components/agents/integrations/ag-ui-setup-guidance";
import { FcpSetupGuidance } from "@/components/agents/integrations/fcp-setup-guidance";
import { SlackSetupGuidance } from "@/components/agents/integrations/slack-setup-guidance";
import { getSlackEndpointManifest } from "@/lib/api/agent-endpoints";
import type {
  A2aChannelConfig,
  AgUiChannelConfig,
  AppChannel,
  FcpChannelConfig,
  SlackChannelConfig,
} from "@/lib/api/types";
import { getEndpointLifecyclePresentation } from "@/lib/app-channels";
type SlackEndpointConfig = SlackChannelConfig & {
  agent_surface_enabled?: boolean;
};

function endpointUrl(endpointId: string, suffix: string): string {
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  return `${origin}/api/v1/e/${endpointId}/${suffix}`;
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
  const [creatingSlackApp, setCreatingSlackApp] = useState(false);
  const [slackManifestError, setSlackManifestError] = useState<string | null>(null);
  const lifecycle = getEndpointLifecyclePresentation(channel);
  const configure = configureHref ? () => router.push(configureHref) : undefined;

  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackEndpointConfig;
    const webhookPath = `/api/v1/e/${channel.id}/slack/events`;
    const webhookUrl = endpointUrl(channel.id, "slack/events");
    const hasSlackConfig =
      !!(config.signing_secret_configured || config.signing_secret) &&
      !!(config.bot_token_configured || config.bot_token);
    const isLocalhost =
      typeof window !== "undefined" &&
      (window.location.hostname === "localhost" || window.location.hostname === "127.0.0.1");

    const createSlackApp = async () => {
      setCreatingSlackApp(true);
      setSlackManifestError(null);
      try {
        const manifest = await getSlackEndpointManifest(channel.id);
        window.open(manifest.create_url, "_blank", "noopener,noreferrer");
      } catch {
        setSlackManifestError("Could not open the Slack app manifest. Try again.");
      } finally {
        setCreatingSlackApp(false);
      }
    };

    return (
      <div className="space-y-3">
        <SetupHeading />
        <SlackSetupGuidance
          hasSlackConfig={hasSlackConfig}
          isPublished={lifecycle.isLive}
          webhookVerified={!!config.webhook_verified_at}
          firstMessageReceived={!!config.first_message_received_at}
          webhookUrl={webhookUrl}
          webhookPath={webhookPath}
          isLocalhost={isLocalhost}
          agentSurfaceEnabled={config.agent_surface_enabled ?? false}
          onCreateSlackApp={() => void createSlackApp()}
          creatingSlackApp={creatingSlackApp}
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
