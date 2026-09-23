"use client";

import { use } from "react";
import { AgentChannelEditor } from "@/components/agents/agent-channel-editor";

export default function EditAgentChannelPage({
  params,
}: {
  params: Promise<{ agentId: string; channelId: string }>;
}) {
  const { agentId, channelId } = use(params);
  return <AgentChannelEditor agentId={agentId} channelId={channelId} />;
}
