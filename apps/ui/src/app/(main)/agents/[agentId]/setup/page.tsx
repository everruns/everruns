"use client";

import { useLocale as useDisplayLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { use } from "react";
import { useSearchParams } from "next/navigation";
import { useAgent } from "@/hooks/use-agents";
import { usePageTitle } from "@/hooks";
import { AgentTemplateSetup } from "@/components/agents/agent-template-setup";
import { PageContainer, PageMasthead, BackLink } from "@/components/layout";
import { getDisplayName } from "@/lib/entity-lifecycle";

/// Guided setup for an agent imported from a template (`?template=<name>`).
export default function AgentSetupPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const { agentId } = use(params);
  const template = useSearchParams()?.get("template") ?? "";
  const { data: agent } = useAgent(agentId);
  usePageTitle(ui("Set up"), ui("Agents"));

  return (
    <PageContainer>
      <BackLink href={`/agents/${agentId}`}>{ui("Back to agent")}</BackLink>
      <PageMasthead
        title={
          agent
            ? ui("Set up {value0}", { value0: getDisplayName(agent, displayLocale) })
            : ui("Set up agent")
        }
      />
      <div className="max-w-2xl">
        <AgentTemplateSetup agentId={agentId} templateName={template} />
      </div>
    </PageContainer>
  );
}
