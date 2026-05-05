"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useCreateApp } from "@/hooks/use-apps";
import { usePageTitle } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AgentSelect } from "@/components/agent/agent-select";
import { AgentIdentitySelect } from "@/components/agent-identity/agent-identity-select";
import { HarnessSelect } from "@/components/harness/harness-select";
import Link from "next/link";
import { ArrowLeft, RefreshCw } from "lucide-react";
import type {
  AgUiToolVisibility,
  ChannelType,
  InvocationSessionMode,
  SessionStrategy,
  SlackReplyMode,
} from "@/lib/api/types";
import {
  getAgUiToolVisibilityDisplayName,
  getChannelTypeDisplayName,
  getInvocationSessionModeDisplayName,
  getSessionStrategyDisplayName,
  getSlackReplyModeDisplayName,
} from "@/lib/app-channels";
import { DEFAULT_AG_UI_GENERIC_TOOL_TEXT } from "@/lib/api/types/app-types";
import { generateChannelToken } from "@/lib/channel-tokens";

export default function NewAppPage() {
  usePageTitle("New App", "Apps");
  const router = useRouter();
  const createApp = useCreateApp();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [agentId, setAgentId] = useState("");
  const [harnessId, setHarnessId] = useState("");
  const [agentIdentityId, setAgentIdentityId] = useState("");
  const [channelType, setChannelType] = useState<ChannelType | "">("");
  const [slackSigningSecret, setSlackSigningSecret] = useState("");
  const [slackBotToken, setSlackBotToken] = useState("");
  const [slackTeamId, setSlackTeamId] = useState("");
  const [slackChannelId, setSlackChannelId] = useState("");
  const [slackSessionStrategy, setSlackSessionStrategy] = useState<SessionStrategy>("per_thread");
  const [slackReplyMode, setSlackReplyMode] = useState<SlackReplyMode>("all_messages");
  const [scheduleCronExpression, setScheduleCronExpression] = useState("0 * * * * * *");
  const [scheduleTimezone, setScheduleTimezone] = useState("UTC");
  const [invocationSessionMode, setInvocationSessionMode] =
    useState<InvocationSessionMode>("shared_session");
  const [channelMessage, setChannelMessage] = useState("");
  const [webhookToken, setWebhookToken] = useState("");
  const [agUiToken, setAgUiToken] = useState("");
  const [agUiToolVisibility, setAgUiToolVisibility] = useState<AgUiToolVisibility>("generic");
  const [agUiGenericToolText, setAgUiGenericToolText] = useState(DEFAULT_AG_UI_GENERIC_TOOL_TEXT);

  const handleChannelTypeChange = (value: string) => {
    const nextChannelType = value === "__none__" ? "" : (value as ChannelType);
    setChannelType(nextChannelType);
    if (nextChannelType === "ag_ui" && channelType !== "ag_ui" && !agUiToken) {
      setAgUiToken(generateChannelToken());
    }
  };

  const buildChannelConfig = () => {
    switch (channelType) {
      case "ag_ui":
        return {
          anonymous: true,
          ...(agUiToken.trim() ? { token: agUiToken.trim() } : {}),
          tool_visibility: agUiToolVisibility,
          generic_tool_text: agUiGenericToolText.trim() || DEFAULT_AG_UI_GENERIC_TOOL_TEXT,
        };
      case "slack":
        return {
          signing_secret: slackSigningSecret,
          bot_token: slackBotToken,
          session_strategy: slackSessionStrategy,
          reply_mode: slackReplyMode,
          ...(slackTeamId ? { team_id: slackTeamId } : {}),
          ...(slackChannelId ? { channel_id: slackChannelId } : {}),
        };
      case "schedule":
        return {
          cron_expression: scheduleCronExpression,
          timezone: scheduleTimezone || "UTC",
          session_mode: invocationSessionMode,
          message: channelMessage,
        };
      case "webhook":
        return {
          token: webhookToken,
          session_mode: invocationSessionMode,
          message: channelMessage,
        };
      default:
        return undefined;
    }
  };

  const isChannelConfigValid =
    channelType === ""
      ? true
      : channelType === "ag_ui"
        ? agUiToolVisibility !== "generic" || agUiGenericToolText.trim().length <= 120
        : channelType === "slack"
          ? !!slackSigningSecret && !!slackBotToken
          : channelType === "schedule"
            ? !!scheduleCronExpression && !!channelMessage
            : !!webhookToken && !!channelMessage;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const app = await createApp.mutateAsync({
        name,
        description: description || undefined,
        agent_id: agentId || undefined,
        agent_identity_id: agentIdentityId || undefined,
        harness_id: harnessId,
        channel_type: channelType || undefined,
        channel_config: channelType ? buildChannelConfig() : undefined,
      });

      // Redirect to the detail page for channel-specific setup.
      router.push(`/apps/${app.id}`);
    } catch (error) {
      console.error("Failed to create app:", error);
    }
  };

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/apps"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Apps
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Create New App</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Support Bot"
                required
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Input
                id="description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Customer support bot for #help channel"
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="harness">Harness</Label>
                <HarnessSelect value={harnessId} onValueChange={setHarnessId} />
              </div>

              <div className="space-y-2">
                <Label htmlFor="agent">Agent (optional)</Label>
                <AgentSelect
                  value={agentId}
                  onValueChange={setAgentId}
                  includeNoneOption
                  noneLabel="Choose later"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="agent_identity">Agent identity</Label>
                <AgentIdentitySelect value={agentIdentityId} onValueChange={setAgentIdentityId} />
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="channel_type">Channel (optional)</Label>
              <Select value={channelType || "__none__"} onValueChange={handleChannelTypeChange}>
                <SelectTrigger id="channel_type">
                  <SelectValue placeholder="Choose later" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">Choose later</SelectItem>
                  <SelectItem value="slack">{getChannelTypeDisplayName("slack")}</SelectItem>
                  <SelectItem value="ag_ui">{getChannelTypeDisplayName("ag_ui")}</SelectItem>
                  <SelectItem value="schedule">{getChannelTypeDisplayName("schedule")}</SelectItem>
                  <SelectItem value="webhook">{getChannelTypeDisplayName("webhook")}</SelectItem>
                </SelectContent>
              </Select>
            </div>

            {channelType === "slack" && (
              <div className="space-y-4 rounded-md border p-4">
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-2">
                    <Label htmlFor="slack_signing_secret">Signing Secret</Label>
                    <Input
                      id="slack_signing_secret"
                      type="password"
                      value={slackSigningSecret}
                      onChange={(e) => setSlackSigningSecret(e.target.value)}
                      placeholder="Slack signing secret"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="slack_bot_token">Bot Token</Label>
                    <Input
                      id="slack_bot_token"
                      type="password"
                      value={slackBotToken}
                      onChange={(e) => setSlackBotToken(e.target.value)}
                      placeholder="xoxb-..."
                    />
                  </div>
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-2">
                    <Label htmlFor="slack_team_id">Workspace ID (optional)</Label>
                    <Input
                      id="slack_team_id"
                      value={slackTeamId}
                      onChange={(e) => setSlackTeamId(e.target.value)}
                      placeholder="T0123456789"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="slack_channel_id">Channel ID (optional)</Label>
                    <Input
                      id="slack_channel_id"
                      value={slackChannelId}
                      onChange={(e) => setSlackChannelId(e.target.value)}
                      placeholder="C0123456789"
                    />
                  </div>
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-2">
                    <Label htmlFor="slack_session_strategy">Session Strategy</Label>
                    <Select
                      value={slackSessionStrategy}
                      onValueChange={(value) => setSlackSessionStrategy(value as SessionStrategy)}
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
                    <Label htmlFor="slack_reply_mode">Reply Mode</Label>
                    <Select
                      value={slackReplyMode}
                      onValueChange={(value) => setSlackReplyMode(value as SlackReplyMode)}
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
                </div>
              </div>
            )}

            {channelType === "ag_ui" && (
              <div className="space-y-4 rounded-md border p-4">
                <div className="space-y-2">
                  <Label htmlFor="ag_ui_token">Endpoint Token</Label>
                  <div className="flex gap-2">
                    <Input
                      id="ag_ui_token"
                      value={agUiToken}
                      onChange={(e) => setAgUiToken(e.target.value)}
                      className="font-mono"
                      placeholder="Generated bearer token"
                    />
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      onClick={() => setAgUiToken(generateChannelToken())}
                      aria-label="Regenerate AG-UI token"
                    >
                      <RefreshCw className="h-4 w-4" />
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    AG-UI clients must send this as `Authorization: Bearer &lt;token&gt;` or
                    `X-Everruns-AG-UI-Token`.
                  </p>
                </div>

                <div className="space-y-2">
                  <Label htmlFor="ag_ui_tool_visibility">Tool Activity</Label>
                  <Select
                    value={agUiToolVisibility}
                    onValueChange={(value) => setAgUiToolVisibility(value as AgUiToolVisibility)}
                  >
                    <SelectTrigger id="ag_ui_tool_visibility">
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
                  <p className="text-xs text-muted-foreground">
                    Tool names, arguments, and results are never sent to public AG-UI clients.
                  </p>
                </div>

                {agUiToolVisibility === "generic" && (
                  <div className="space-y-2">
                    <Label htmlFor="ag_ui_generic_tool_text">Generic Activity Text</Label>
                    <Input
                      id="ag_ui_generic_tool_text"
                      value={agUiGenericToolText}
                      onChange={(e) => setAgUiGenericToolText(e.target.value)}
                      maxLength={120}
                      placeholder={DEFAULT_AG_UI_GENERIC_TOOL_TEXT}
                    />
                  </div>
                )}
              </div>
            )}

            {channelType === "schedule" && (
              <div className="space-y-4 rounded-md border p-4">
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-2">
                    <Label htmlFor="schedule_cron_expression">Cron Expression</Label>
                    <Input
                      id="schedule_cron_expression"
                      value={scheduleCronExpression}
                      onChange={(e) => setScheduleCronExpression(e.target.value)}
                      placeholder="0 * * * * * *"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="schedule_timezone">Timezone</Label>
                    <Input
                      id="schedule_timezone"
                      value={scheduleTimezone}
                      onChange={(e) => setScheduleTimezone(e.target.value)}
                      placeholder="UTC"
                    />
                  </div>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="schedule_session_mode">Invocation Session Mode</Label>
                  <Select
                    value={invocationSessionMode}
                    onValueChange={(value) =>
                      setInvocationSessionMode(value as InvocationSessionMode)
                    }
                  >
                    <SelectTrigger id="schedule_session_mode">
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
                <div className="space-y-2">
                  <Label htmlFor="schedule_message">Invocation Message</Label>
                  <Textarea
                    id="schedule_message"
                    value={channelMessage}
                    onChange={(e) => setChannelMessage(e.target.value)}
                    placeholder="Run repository checks for {{app.name}}"
                  />
                </div>
              </div>
            )}

            {channelType === "webhook" && (
              <div className="space-y-4 rounded-md border p-4">
                <div className="space-y-2">
                  <Label htmlFor="webhook_token">Webhook Token</Label>
                  <Input
                    id="webhook_token"
                    type="password"
                    value={webhookToken}
                    onChange={(e) => setWebhookToken(e.target.value)}
                    placeholder="shared-secret"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="webhook_session_mode">Invocation Session Mode</Label>
                  <Select
                    value={invocationSessionMode}
                    onValueChange={(value) =>
                      setInvocationSessionMode(value as InvocationSessionMode)
                    }
                  >
                    <SelectTrigger id="webhook_session_mode">
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
                <div className="space-y-2">
                  <Label htmlFor="webhook_message">Invocation Message</Label>
                  <Textarea
                    id="webhook_message"
                    value={channelMessage}
                    onChange={(e) => setChannelMessage(e.target.value)}
                    placeholder="Process webhook payload for {{payload.repo.name}}"
                  />
                </div>
              </div>
            )}

            <p className="text-sm text-muted-foreground">
              {channelType === "slack"
                ? "Slack needs credentials up front. You can still refine webhook setup and manifest details on the app detail page."
                : channelType === "ag_ui"
                  ? "After creating the app, publish it and point your AG-UI client at the channel endpoint on the detail page."
                  : channelType === "schedule"
                    ? "This creates an app-level scheduled invocation channel. It activates only when the app is published."
                    : channelType === "webhook"
                      ? "This creates an authenticated app webhook channel. The detail page will show the endpoint URL to call."
                      : `Create the draft now, then assign an agent or add ${getChannelTypeDisplayName("slack")}, ${getChannelTypeDisplayName("ag_ui")}, ${getChannelTypeDisplayName("schedule")}, or ${getChannelTypeDisplayName("webhook")} later from the detail page.`}
            </p>

            <div className="flex gap-4">
              <Button
                type="submit"
                disabled={createApp.isPending || !name || !harnessId || !isChannelConfigValid}
              >
                {createApp.isPending ? "Creating..." : "Create App"}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
                Cancel
              </Button>
            </div>

            {createApp.error && (
              <p className="text-sm text-destructive">Error: {createApp.error.message}</p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
