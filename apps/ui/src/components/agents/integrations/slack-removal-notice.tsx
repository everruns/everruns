import type { AgentChannel } from "@/lib/api/types";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";

export function SlackRemovalNotice({
  channels,
  action,
}: {
  channels: AgentChannel[];
  action: "archive" | "delete" | "channel";
}) {
  const slack = channels.filter((channel) => channel.channel_type === "slack");
  if (slack.length === 0) return null;
  const managed = slack.some(
    (channel) => (channel.channel_config as Record<string, unknown>)?.slack_app_provisioned,
  );
  const manual = slack.some((channel) => {
    const config = channel.channel_config as Record<string, unknown>;
    return (
      !config?.slack_app_provisioned &&
      (config?.bot_token_configured || config?.signing_secret_configured)
    );
  });
  return (
    <Notice variant="warning">
      <NoticeTitle>Slack connection</NoticeTitle>
      <NoticeDescription className="space-y-2">
        <p>
          {action === "channel"
            ? "New Slack messages to this channel will stop reaching this agent."
            : "New Slack messages will stop reaching this agent."}
        </p>
        {managed && (
          <p>
            {action === "channel"
              ? "The Slack app for this channel will be removed from Slack, including its bot and workspace installation."
              : "The Slack apps created for this agent will be removed from Slack, including their bots and workspace installations."}
          </p>
        )}
        {managed && action === "archive" && (
          <p>
            Restoring the agent will not restore its Slack apps. You will need to reinstall them.
          </p>
        )}
        {manual && (
          <p>
            This agent has a manually configured Slack app. Remove its app manually in Slack;
            Everruns cannot remove it automatically.
          </p>
        )}
      </NoticeDescription>
    </Notice>
  );
}
