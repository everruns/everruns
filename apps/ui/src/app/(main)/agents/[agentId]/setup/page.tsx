"use client";

import { use } from "react";
import { useSearchParams } from "next/navigation";
import { useAgent } from "@/hooks/use-agents";
import { usePageTitle } from "@/hooks";
import { AgentTemplateSetup } from "@/components/agents/agent-template-setup";
import { PageContainer, PageMasthead, BackLink } from "@/components/layout";
import { getDisplayName } from "@/lib/entity-lifecycle";

/// Guided setup for an agent imported from a template (`?template=<name>`).
export default function AgentSetupPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const template = useSearchParams()?.get("template") ?? "";
  const { data: agent } = useAgent(agentId);
  usePageTitle("Set up", "Agents");

  return (
    <PageContainer>
      <BackLink href={`/agents/${agentId}`}>Back to agent</BackLink>
      <PageMasthead title={agent ? `Set up ${getDisplayName(agent)}` : "Set up agent"} />
      <div className="max-w-2xl">
        <AgentTemplateSetup agentId={agentId} templateName={template} />
      </div>
    </PageContainer>
  );
}
