"use client";

import { use } from "react";
import { AgentEndpointEditor } from "@/components/agents/agent-endpoint-editor";

export default function EditAgentEndpointPage({
  params,
}: {
  params: Promise<{ agentId: string; endpointId: string }>;
}) {
  const { agentId, endpointId } = use(params);
  return <AgentEndpointEditor agentId={agentId} endpointId={endpointId} />;
}
