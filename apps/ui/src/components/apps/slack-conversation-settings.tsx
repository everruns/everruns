"use client";

import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { SessionStrategy, SlackReplyMode, SlackResponsePolicy } from "@/lib/api/types";
import { getSessionStrategyDisplayName, getSlackReplyModeDisplayName } from "@/lib/channel-display";

const SLACK_SESSION_DESCRIPTIONS: Record<SessionStrategy, string> = {
  per_thread: "Each Slack thread has its own session and conversation history.",
  per_channel:
    "Channel conversations share one session and history across all threads in that channel.",
  per_user: "Each person shares one session across their channel conversations with this channel.",
};

export function SlackConversationSettings({
  idPrefix = "slack",
  sessionStrategy,
  replyMode,
  onSessionStrategyChange,
  onReplyModeChange,
  responsePolicy,
  onResponsePolicyChange,
}: {
  idPrefix?: string;
  sessionStrategy: SessionStrategy;
  replyMode: SlackReplyMode;
  responsePolicy: SlackResponsePolicy;
  onResponsePolicyChange: (value: SlackResponsePolicy) => void;
  onSessionStrategyChange: (value: SessionStrategy) => void;
  onReplyModeChange: (value: SlackReplyMode) => void;
}) {
  return (
    <section className="space-y-4" aria-labelledby={`${idPrefix}_conversation_heading`}>
      <div className="space-y-1">
        <h3 id={`${idPrefix}_conversation_heading`} className="text-sm font-medium">
          Conversation behavior
        </h3>
        <p className="text-xs leading-relaxed text-muted-foreground">
          Choose how conversations share context and what the agent posts to Slack.
        </p>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        <div className="min-w-0 space-y-2">
          <Label htmlFor={`${idPrefix}_session_strategy`}>Session strategy</Label>
          <Select
            value={sessionStrategy}
            onValueChange={(value) => onSessionStrategyChange(value as SessionStrategy)}
          >
            <SelectTrigger
              id={`${idPrefix}_session_strategy`}
              className="w-full"
              aria-describedby={`${idPrefix}_session_strategy_description`}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="per_thread">
                {getSessionStrategyDisplayName("per_thread")}
              </SelectItem>
              <SelectItem value="per_channel">
                {getSessionStrategyDisplayName("per_channel")}
              </SelectItem>
              <SelectItem value="per_user">{getSessionStrategyDisplayName("per_user")}</SelectItem>
            </SelectContent>
          </Select>
          <p
            id={`${idPrefix}_session_strategy_description`}
            className="text-xs leading-relaxed text-muted-foreground"
          >
            {SLACK_SESSION_DESCRIPTIONS[sessionStrategy]}
          </p>
        </div>
        <div className="min-w-0 space-y-2">
          <Label htmlFor={`${idPrefix}_reply_mode`}>Reply mode</Label>
          <Select
            value={replyMode}
            onValueChange={(value) => onReplyModeChange(value as SlackReplyMode)}
          >
            <SelectTrigger
              id={`${idPrefix}_reply_mode`}
              className="w-full"
              aria-describedby={`${idPrefix}_reply_mode_description`}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all_messages">
                {getSlackReplyModeDisplayName("all_messages")}
              </SelectItem>
              <SelectItem value="tool_only">{getSlackReplyModeDisplayName("tool_only")}</SelectItem>
            </SelectContent>
          </Select>
          <p
            id={`${idPrefix}_reply_mode_description`}
            className="text-xs leading-relaxed text-muted-foreground"
          >
            {replyMode === "all_messages" ? (
              "Automatically post every assistant response to Slack. Replies stream in the agent pane."
            ) : (
              <>
                The agent chooses when to send updates, questions, and answers. Other assistant
                messages stay in Everruns. Slack acknowledges each request while the agent works.
              </>
            )}
          </p>
        </div>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}_response_policy`}>Response policy</Label>
        <Select
          value={responsePolicy}
          onValueChange={(value) => onResponsePolicyChange(value as SlackResponsePolicy)}
        >
          <SelectTrigger
            id={`${idPrefix}_response_policy`}
            className="w-full"
            aria-describedby={`${idPrefix}_response_policy_description`}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all_messages">All messages</SelectItem>
            <SelectItem value="mentions_only">Mentions only</SelectItem>
            <SelectItem value="relevant_messages">Relevant messages</SelectItem>
          </SelectContent>
        </Select>
        <p
          id={`${idPrefix}_response_policy_description`}
          className="text-xs leading-relaxed text-muted-foreground"
        >
          {responsePolicy === "all_messages"
            ? "Respond to every message received by this channel."
            : responsePolicy === "mentions_only"
              ? "Respond only to direct messages and @mentions."
              : "Respond to direct messages, @mentions, and clear requests within the agent’s purpose. Stay silent on unrelated or uncertain messages."}
        </p>
      </div>
    </section>
  );
}

export function SlackAgentPaneSettings({
  enabled,
  configured,
  onChange,
  idPrefix = "slack",
}: {
  enabled: boolean;
  configured: boolean;
  onChange: (enabled: boolean) => void;
  idPrefix?: string;
}) {
  return (
    <div className="space-y-2 border-t pt-4">
      <div className="flex items-center justify-between gap-4">
        <Label htmlFor={`${idPrefix}_agent_surface`}>Slack agent pane</Label>
        <Switch id={`${idPrefix}_agent_surface`} checked={enabled} onCheckedChange={onChange} />
      </div>
      <p className="text-xs text-muted-foreground">
        Show this agent in Slack’s agent pane alongside its channel bot. Requires the
        assistant:write permission.
      </p>
      {enabled && configured && (
        <p className="text-xs text-muted-foreground">
          Save changes, update the existing Slack app with a fresh manifest under Configure
          manually, and reinstall it to grant the permission.
        </p>
      )}
    </div>
  );
}
