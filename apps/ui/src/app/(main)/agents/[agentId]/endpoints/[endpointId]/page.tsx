"use client";

import { use } from "react";
import { AgentEndpointEditor } from "@/components/agents/agent-endpoint-editor";

export default function EditAgentEndpointPage({
  params,
  searchParams,
}: {
  params: Promise<{ agentId: string; endpointId: string }>;
  searchParams: Promise<{ slack_install?: string; reason?: string }>;
}) {
  const { agentId, endpointId } = use(params);
  const install = use(searchParams);
  return (
    <AgentEndpointEditor
      agentId={agentId}
      endpointId={endpointId}
      slackInstallFailure={install.slack_install === "failed" ? (install.reason ?? "") : undefined}
    />
  );
}
