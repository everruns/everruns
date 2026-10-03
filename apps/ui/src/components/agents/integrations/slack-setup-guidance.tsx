"use client";

import { useQuery } from "@tanstack/react-query";
import { CircleCheck, Circle, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/ui/code-block";
import { ChannelUsePanel } from "@/components/agents/channel-use-panel";
import { getSlackChannelManifest } from "@/lib/api/agent-channels";
import { getChannelLifecyclePresentation } from "@/lib/channel-display";
import { isPublicHttpsUrl } from "@/lib/public-origin";
import type { AgentChannel, SlackChannelConfig } from "@/lib/api/types";

export type SlackChannelSetupConfig = SlackChannelConfig & {
  slack_app_id?: string;
  agent_surface_enabled?: boolean;
};

export function hasSlackCredentials(config: SlackChannelConfig): boolean {
  return (
    !!(config.signing_secret_configured || config.signing_secret) &&
    !!(config.bot_token_configured || config.bot_token)
  );
}

export function slackConnectionLabel(config: SlackChannelConfig): string {
  if (!hasSlackCredentials(config)) return "Not connected";
  if (config.first_message_received_at) return "Message received";
  if (config.webhook_verified_at) return "Request URL verified";
  return "Credentials saved";
}

export function SlackConnectionStatus({ channel }: { channel: AgentChannel }) {
  const config = channel.channel_config as SlackChannelConfig;
  const configured = hasSlackCredentials(config);
  const received = !!config.first_message_received_at;
  const live = getChannelLifecyclePresentation(channel).isLive;
  const Icon = received ? CircleCheck : Circle;
  return (
    <section className="space-y-2" aria-label="Slack connection">
      <h3 className="text-sm font-medium">Slack connection</h3>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Icon className={received ? "size-4 text-success" : "size-4 text-muted-foreground"} />
        <span className="font-medium">{slackConnectionLabel(config)}</span>
        {config.team_id && <span className="text-xs text-muted-foreground">{config.team_id}</span>}
      </div>
      <p className="text-xs text-muted-foreground">
        {!live
          ? "Publish this channel to receive Slack messages. Installation and publication are separate."
          : !configured
            ? "Add this agent to Slack or use credentials from an existing Slack app below."
            : received
              ? // Signed message delivery is stronger evidence than the optional URL challenge.
                "A signed Slack message has reached this channel."
              : config.webhook_verified_at
                ? "Slack verified the Request URL. Invite the bot to a channel, then @mention it to test."
                : "No Slack message received yet. Invite the bot to a channel, then @mention it to test."}
      </p>
    </section>
  );
}

// Mounted only inside the manual disclosure; normal installation needs no manifest or URL.
export function SlackManualSetup({ channel }: { channel: AgentChannel }) {
  const configured = hasSlackCredentials(channel.channel_config as SlackChannelConfig);
  const live = getChannelLifecyclePresentation(channel).isLive;
  const manifest = useQuery({
    queryKey: ["slack-channel-manifest", channel.id, channel.updated_at],
    queryFn: () => getSlackChannelManifest(channel.id),
    enabled: live || configured,
  });
  const requestUrl = manifest.data?.manifest_yaml.match(
    /event_subscriptions:\s*\n\s*request_url:\s*"([^"]+)"/,
  )?.[1];
  const canCreate = live && isPublicHttpsUrl(requestUrl ?? null);
  return (
    <div className="space-y-3">
      {!configured && (
        <>
          <p className="text-xs text-muted-foreground">
            {live
              ? "Create an app from this channel’s manifest in Slack, install it to your workspace, then enter its signing secret and bot token below."
              : "Publish this channel before creating a Slack app from its manifest. Slack checks the Request URL during app creation."}
          </p>
          {live && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={!canCreate || !manifest.data}
              onClick={() =>
                manifest.data &&
                window.open(manifest.data.create_url, "_blank", "noopener,noreferrer")
              }
            >
              <ExternalLink className="size-3" /> Create Slack app
            </Button>
          )}
        </>
      )}
      {manifest.isLoading && (
        <p className="text-xs text-muted-foreground">Loading the Slack app manifest…</p>
      )}
      {manifest.isError && (
        <div role="alert" className="space-y-2">
          <p className="text-xs text-destructive">Could not load the Slack app manifest.</p>
          <Button type="button" size="sm" variant="outline" onClick={() => manifest.refetch()}>
            Try again
          </Button>
        </div>
      )}
      {manifest.data && !isPublicHttpsUrl(requestUrl ?? null) && (
        <p className="text-xs text-destructive">
          Set PUBLIC_APP_URL to a public HTTPS origin, restart Everruns, and reload before creating
          the Slack app.
        </p>
      )}
      <ChannelUsePanel channel={channel} />
      {configured && manifest.data && (
        <details>
          <summary className="cursor-pointer text-sm">App manifest</summary>
          <p className="my-2 text-xs text-muted-foreground">
            Save settings first to generate an updated manifest. Update your existing app in Slack
            and reinstall it when permissions change.
          </p>
          <CodeBlock
            samples={[
              { label: "Slack app manifest", language: "yaml", code: manifest.data.manifest_yaml },
            ]}
          />
        </details>
      )}
    </div>
  );
}
