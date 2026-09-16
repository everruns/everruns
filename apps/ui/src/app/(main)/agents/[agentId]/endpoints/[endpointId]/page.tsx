"use client";

import { use } from "react";
import { useAgent } from "@/hooks/use-agents";
import { useAgentEndpoints } from "@/hooks/use-agent-endpoints";
import { ChannelEditor } from "@/components/apps/channel-editor";
import { ResourceNotFound } from "@/components/resource-not-found";
import { getDisplayName } from "@/lib/entity-lifecycle";

export default function EditAgentEndpointPage({
  params,
}: {
  params: Promise<{ agentId: string; endpointId: string }>;
}) {
  const { agentId, endpointId } = use(params);
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { endpoints, isLoading } = useAgentEndpoints(agentId);

  // The endpoint knows which App row still owns it; every write addresses that
  // App until EVE-1011 removes the indirection.
  const owner = endpoints.find(({ channel }) => channel.id === endpointId);

  if (isLoading || agentLoading)
    return <div className="container mx-auto p-6">Loading endpoint...</div>;

  if (!agent || !owner) {
    return (
      <ResourceNotFound
        title="Endpoint not found"
        description="This endpoint may have been deleted, moved to another agent, or the URL may be wrong."
        backHref={`/agents/${agentId}?tab=integrations`}
        backLabel="Back to agent"
        resourceId={endpointId}
      />
    );
  }

  const agentName = getDisplayName(agent);
  const returnHref = `/agents/${agentId}?tab=integrations`;

  return (
    <ChannelEditor
      appId={owner.app.id}
      channelId={endpointId}
      nav={{
        breadcrumbs: [
          { label: "Agents", href: "/agents" },
          { label: agentName, href: returnHref },
        ],
        returnHref,
        returnLabel: agentName,
      }}
    />
  );
}
