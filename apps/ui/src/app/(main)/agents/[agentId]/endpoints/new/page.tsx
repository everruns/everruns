"use client";

import { use, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Check, Radio } from "lucide-react";
import { useAgent } from "@/hooks/use-agents";
import { useCreateAgentEndpoint } from "@/hooks/use-agent-endpoints";
import { usePolicies } from "@/hooks/use-policies";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
import {
  buildChannelConfig,
  ChannelForm,
  ChannelFormSummary,
  ChannelTypePicker,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/apps/channel-form";
import {
  BackLink,
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageFooter,
  PageMain,
  PageMasthead,
  PageRail,
} from "@/components/layout";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";

export default function NewAgentEndpointPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const router = useRouter();
  const { data: agent, isLoading } = useAgent(agentId);
  const { can, isLoading: policiesLoading } = usePolicies("agents");
  const createEndpoint = useCreateAgentEndpoint(agentId);
  const [formState, setFormState] = useState(() => getDefaultChannelFormState("webhook"));
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const canManage = !policiesLoading && can("agent.manage") && !isReadOnlyStatus(agent?.status);

  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(returnHref);
  }, [agent, canManage, policiesLoading, returnHref, router]);

  if (isLoading || policiesLoading) {
    return <div className="container mx-auto p-6">Loading endpoint form...</div>;
  }
  if (!agent) {
    return (
      <ResourceNotFound
        title="Agent not found"
        description="This agent may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/agents"
        backLabel="Back to agents"
        resourceId={agentId}
      />
    );
  }

  const agentName = getDisplayName(agent);

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agents", href: "/agents" },
          { label: agentName, href: returnHref },
          { label: "New endpoint" },
        ]}
      />
      <PageMasthead
        icon={<Radio />}
        title="New endpoint"
        description="Expose this agent through a webhook, AG-UI, Slack, or another transport."
        actions={
          <>
            <Button
              type="submit"
              form="endpoint-edit-form"
              disabled={!canManage || !isChannelFormValid(formState) || createEndpoint.isPending}
            >
              <Check className="size-4" />
              {createEndpoint.isPending ? "Saving..." : "Save endpoint"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.push(returnHref)}>
              Discard
            </Button>
          </>
        }
      />
      <form
        id="endpoint-edit-form"
        onSubmit={(event) => {
          event.preventDefault();
          createEndpoint.mutate(
            {
              channel_type: formState.kind,
              channel_config: buildChannelConfig(formState),
              enabled: formState.enabled,
            },
            {
              onSuccess: (endpoint) => router.push(`/agents/${agentId}/endpoints/${endpoint.id}`),
            },
          );
        }}
      >
        <PageColumns>
          <PageMain>
            <Card>
              <CardHeader>
                <CardTitle>1. Endpoint type</CardTitle>
              </CardHeader>
              <CardContent>
                <ChannelTypePicker
                  value={formState.kind}
                  onChange={(kind) => setFormState(getDefaultChannelFormState(kind))}
                />
              </CardContent>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>
                  2. Configure {formState.kind === "ag_ui" ? "AG-UI" : formState.kind}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ChannelForm state={formState} onChange={setFormState} mode="new" />
              </CardContent>
            </Card>
          </PageMain>
          <PageRail>
            <ChannelFormSummary state={formState} />
          </PageRail>
        </PageColumns>
      </form>
      <PageFooter>
        <BackLink href={returnHref}>Back to {agentName}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
