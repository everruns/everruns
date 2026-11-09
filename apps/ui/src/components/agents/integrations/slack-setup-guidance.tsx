"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useQuery } from "@tanstack/react-query";
import { CircleCheck, Circle, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/ui/code-block";
import { ChannelUsePanel } from "@/components/agents/channel-use-panel";
import { useSlackWorkspaceName } from "@/hooks/use-agent-channels";
import { getSlackChannelManifest } from "@/lib/api/agent-channels";
import { getChannelLifecyclePresentation } from "@/lib/channel-display";
import { isPublicHttpsUrl } from "@/lib/public-origin";
import type { AgentChannel, SlackChannelConfig } from "@/lib/api/types";

export type SlackChannelSetupConfig = SlackChannelConfig & {
  slack_app_id?: string;
  agent_surface_enabled?: boolean;
};

/** Socket Mode needs the app-level token; the Request URL needs the signing secret. */
export function hasSlackCredentials(config: SlackChannelConfig): boolean {
  const ingress =
    config.connection === "socket_mode"
      ? config.app_token_configured || config.app_token
      : config.signing_secret_configured || config.signing_secret;
  return !!ingress && !!(config.bot_token_configured || config.bot_token);
}

export function slackConnectionLabel(config: SlackChannelConfig): string {
  if (!hasSlackCredentials(config)) return "Not connected";
  if (config.first_message_received_at) return "Message received";
  // Slack checked the Request URL with a signed challenge, or accepted the Socket Mode
  // connection, so Slack can reach the channel.
  if (config.webhook_verified_at) return "Connected";
  return "Credentials saved";
}

export function SlackConnectionStatus({ channel }: { channel: AgentChannel }) {
  const ui = useUiTranslations();
  const config = channel.channel_config as SlackChannelConfig;
  const configured = hasSlackCredentials(config);
  const received = !!config.first_message_received_at;
  const live = getChannelLifecyclePresentation(channel).isLive;
  const connected = configured && (received || !!config.webhook_verified_at);
  const workspaceName = useSlackWorkspaceName(config.team_id);
  const Icon = connected ? CircleCheck : Circle;
  return (
    <section className="space-y-2" aria-label={ui("Slack connection")}>
      <h3 className="text-sm font-medium">{ui("Slack connection")}</h3>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Icon className={connected ? "size-4 text-success" : "size-4 text-muted-foreground"} />
        <span className="font-medium">{slackConnectionLabel(config)}</span>
        {workspaceName && <span className="text-xs text-muted-foreground">{workspaceName}</span>}
      </div>
      <p className="text-xs text-muted-foreground">
        {!live
          ? ui(
              "Publish this channel to receive Slack messages. Installation and publication are separate.",
            )
          : !configured
            ? ui("Add this agent to Slack or use credentials from an existing Slack app below.")
            : received
              ? // Signed message delivery is stronger evidence than the optional URL challenge.
                ui("A signed Slack message has reached this channel.")
              : config.webhook_verified_at
                ? config.connection === "socket_mode"
                  ? ui(
                      "Everruns is connected to Slack. Invite the bot to a channel, then @mention it to test.",
                    )
                  : ui(
                      "Slack verified the Request URL. Invite the bot to a channel, then @mention it to test.",
                    )
                : ui(
                    "No Slack message received yet. Invite the bot to a channel, then @mention it to test.",
                  )}
      </p>
    </section>
  );
}

// Mounted only inside the manual disclosure; normal installation needs no manifest or URL.
export function SlackManualSetup({ channel }: { channel: AgentChannel }) {
  const ui = useUiTranslations();
  const config = channel.channel_config as SlackChannelConfig;
  const configured = hasSlackCredentials(config);
  const socketMode = config.connection === "socket_mode";
  const live = getChannelLifecyclePresentation(channel).isLive;
  const manifest = useQuery({
    queryKey: ["slack-channel-manifest", channel.id, channel.updated_at],
    queryFn: () => getSlackChannelManifest(channel.id),
    enabled: live || configured,
  });
  const requestUrl = manifest.data?.manifest_yaml.match(
    /event_subscriptions:\s*\n\s*request_url:\s*"([^"]+)"/,
  )?.[1];
  // Socket Mode apps have no Request URL, so they need no public address.
  const reachable = socketMode || isPublicHttpsUrl(requestUrl ?? null);
  const canCreate = live && reachable;
  return (
    <div className="space-y-3">
      {!configured && (
        <>
          <p className="text-xs text-muted-foreground">
            {live && socketMode
              ? ui(
                  "Create an app from this channel’s manifest in Slack and install it to your workspace. Then generate an app-level token and enter it with the bot token below.",
                )
              : live
                ? ui(
                    "Create an app from this channel’s manifest in Slack, install it to your workspace, then enter its signing secret and bot token below.",
                  )
                : ui(
                    "Publish this channel before creating a Slack app from its manifest. Slack checks the Request URL during app creation.",
                  )}
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
              <ExternalLink className="size-3" /> {ui("Create Slack app")}
            </Button>
          )}
        </>
      )}
      {manifest.isLoading && (
        <p className="text-xs text-muted-foreground">{ui("Loading the Slack app manifest…")}</p>
      )}
      {manifest.isError && (
        <div role="alert" className="space-y-2">
          <p className="text-xs text-destructive">{ui("Could not load the Slack app manifest.")}</p>
          <Button type="button" size="sm" variant="outline" onClick={() => manifest.refetch()}>
            {ui("Try again")}
          </Button>
        </div>
      )}
      {manifest.data && !reachable && (
        <p className="text-xs text-destructive">
          {ui(
            "Set PUBLIC_APP_URL to a public HTTPS origin, restart Everruns, and reload before creating the Slack app.",
          )}
        </p>
      )}
      <ChannelUsePanel channel={channel} />
      {configured && manifest.data && (
        <details>
          <summary className="cursor-pointer text-sm">{ui("App manifest")}</summary>
          <p className="my-2 text-xs text-muted-foreground">
            {ui(
              "Save settings first to generate an updated manifest. Update your existing app in Slack and reinstall it when permissions change.",
            )}
          </p>
          <CodeBlock
            samples={[
              {
                label: ui("Slack app manifest"),
                language: "yaml",
                code: manifest.data.manifest_yaml,
              },
            ]}
          />
        </details>
      )}
    </div>
  );
}
