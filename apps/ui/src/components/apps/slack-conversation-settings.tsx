"use client";

import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { SessionStrategy, SlackReplyMode, SlackResponsePolicy } from "@/lib/api/types";
import {
  getSessionStrategyDisplayName,
  getSlackReplyModeDisplayName,
} from "@/lib/endpoint-display";

const SLACK_SESSION_DESCRIPTIONS: Record<SessionStrategy, string> = {
  per_thread: "Each Slack thread has its own session and conversation history.",
  per_channel:
    "Channel conversations share one session and history across all threads in that channel.",
  per_user: "Each person shares one session across their channel conversations with this endpoint.",
};

export function SlackConversationSettings({
  sessionStrategy,
  replyMode,
  onSessionStrategyChange,
  onReplyModeChange,
  responsePolicy,
  onResponsePolicyChange,
}: {
  sessionStrategy: SessionStrategy;
  replyMode: SlackReplyMode;
  responsePolicy: SlackResponsePolicy;
  onResponsePolicyChange: (value: SlackResponsePolicy) => void;
  onSessionStrategyChange: (value: SessionStrategy) => void;
  onReplyModeChange: (value: SlackReplyMode) => void;
}) {
  return (
    <section className="space-y-4" aria-labelledby="slack_conversation_heading">
      <div className="space-y-1">
        <h3 id="slack_conversation_heading" className="text-sm font-medium">
          Conversation behavior
        </h3>
        <p className="text-xs leading-relaxed text-muted-foreground">
          Choose how conversations share context and what the agent posts to Slack.
        </p>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        <div className="min-w-0 space-y-2">
          <Label htmlFor="slack_session_strategy">Session strategy</Label>
          <Select
            value={sessionStrategy}
            onValueChange={(value) => onSessionStrategyChange(value as SessionStrategy)}
          >
            <SelectTrigger
              id="slack_session_strategy"
              className="w-full"
              aria-describedby="slack_session_strategy_description"
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
            id="slack_session_strategy_description"
            className="text-xs leading-relaxed text-muted-foreground"
          >
            {SLACK_SESSION_DESCRIPTIONS[sessionStrategy]}
          </p>
        </div>
        <div className="min-w-0 space-y-2">
          <Label htmlFor="slack_reply_mode">Reply mode</Label>
          <Select
            value={replyMode}
            onValueChange={(value) => onReplyModeChange(value as SlackReplyMode)}
          >
            <SelectTrigger
              id="slack_reply_mode"
              className="w-full"
              aria-describedby="slack_reply_mode_description"
            >
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
          <p
            id="slack_reply_mode_description"
            className="text-xs leading-relaxed text-muted-foreground"
          >
            {replyMode === "all_messages" ? (
              "Post every assistant response to the Slack conversation."
            ) : (
              <>
                Post only updates sent through <code>report_progress</code>. Regular assistant
                messages stay in Everruns.
              </>
            )}
          </p>
        </div>
      </div>
      <div className="space-y-2">
        <Label htmlFor="slack_response_policy">Response policy</Label>
        <Select
          value={responsePolicy}
          onValueChange={(value) => onResponsePolicyChange(value as SlackResponsePolicy)}
        >
          <SelectTrigger
            id="slack_response_policy"
            className="w-full"
            aria-describedby="slack_response_policy_description"
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
          id="slack_response_policy_description"
          className="text-xs leading-relaxed text-muted-foreground"
        >
          {responsePolicy === "all_messages"
            ? "Respond to every message received by this endpoint."
            : responsePolicy === "mentions_only"
              ? "Respond only to direct messages and @mentions."
              : "Respond to direct messages, @mentions, and clear requests within the agent’s purpose. Stay silent on unrelated or uncertain messages."}
        </p>
      </div>
    </section>
  );
}
