"use client";

import { use } from "react";
import { AgentChannelEditor } from "@/components/agents/agent-channel-editor";

export default function EditAgentChannelPage({
  params,
  searchParams,
}: {
  params: Promise<{ agentId: string; channelId: string }>;
  searchParams: Promise<{ slack_install?: string; reason?: string }>;
}) {
  const { agentId, channelId } = use(params);
  const install = use(searchParams);
  return (
    <AgentChannelEditor
      agentId={agentId}
      channelId={channelId}
      slackInstallFailure={install.slack_install === "failed" ? (install.reason ?? "") : undefined}
    />
  );
}
