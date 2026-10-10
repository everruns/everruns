"use client";

import { useRouter } from "next/navigation";
import { ChannelUsePanel } from "@/components/agents/channel-use-panel";
import { A2aSetupGuidance } from "@/components/agents/integrations/a2a-setup-guidance";
import { AgUiSetupGuidance } from "@/components/agents/integrations/ag-ui-setup-guidance";
import { FcpSetupGuidance } from "@/components/agents/integrations/fcp-setup-guidance";
import { SlackChannelConfiguration } from "./slack-channel-configuration";
import { VoiceTalkButton } from "@/components/agents/channels/voice-talk-button";
import type {
  A2aChannelConfig,
  AgUiChannelConfig,
  AgentChannel,
  FcpChannelConfig,
} from "@/lib/api/types";
import { getChannelLifecyclePresentation } from "@/lib/channel-display";
function channelUrl(channelId: string, suffix: string): string {
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  return `${origin}/api/v1/channels/${channelId}/${suffix}`;
}
function SetupHeading() {
  return <p className="text-xs font-medium uppercase text-muted-foreground">Set up</p>;
}

function ChannelSetupGuidance({
  agentId,
  agentName,
  agentDescription,
  channel,
  configureHref,
}: {
  agentId: string;
  agentName: string;
  agentDescription?: string | null;
  channel: AgentChannel;
  configureHref?: string;
}) {
  const router = useRouter();
  const lifecycle = getChannelLifecyclePresentation(channel);
  const configure = configureHref ? () => router.push(configureHref) : undefined;

  if (channel.channel_type === "a2a") {
    const config = channel.channel_config as A2aChannelConfig;
    const url = channelUrl(channel.id, "a2a");
    return (
      <div className="space-y-3">
        <SetupHeading />
        <A2aSetupGuidance
          channelUrl={url}
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
          channelUrl={channelUrl(channel.id, "ag-ui")}
          imageUploadUrl={channelUrl(channel.id, "ag-ui/images")}
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
          channelUrl={channelUrl(channel.id, "fcp")}
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

  if (channel.channel_type === "voice") {
    return (
      <div className="space-y-3">
        <SetupHeading />
        <p className="text-sm text-muted-foreground">
          Voice calls need an OpenAI provider in this organization. Call the agent with the button
          below, the microphone in its chat, or the API with an organization API key. The agent
          writes every answer; the speech model only listens and speaks.
        </p>
        <VoiceTalkButton
          agentId={agentId}
          channelId={channel.id}
          disabled={!lifecycle.isLive}
        />
      </div>
    );
  }

  return null;
}

export function ChannelDetailsPanel({
  agentId,
  agentName,
  agentDescription,
  channel,
  configureHref,
}: {
  agentId: string;
  agentName: string;
  agentDescription?: string | null;
  channel: AgentChannel;
  configureHref?: string;
}) {
  if (channel.channel_type === "slack") {
    return (
      <SlackChannelConfiguration agentId={agentId} channel={channel} canManage={!!configureHref} />
    );
  }
  return (
    <div className="space-y-6">
      <ChannelSetupGuidance
        agentId={agentId}
        agentName={agentName}
        agentDescription={agentDescription}
        channel={channel}
        configureHref={configureHref}
      />
      <ChannelUsePanel channel={channel} agentId={agentId} />
    </div>
  );
}
