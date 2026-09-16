"use client";

import { use, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Radio } from "lucide-react";
import { useAgent } from "@/hooks/use-agents";
import { useApps } from "@/hooks/use-apps";
import { usePolicies } from "@/hooks/use-policies";
import { addChannel, createApp } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
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
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";

export default function NewAgentEndpointPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data: agent, isLoading } = useAgent(agentId);
  const { data: apps, isLoading: appsLoading } = useApps();
  const { can, isLoading: policiesLoading } = usePolicies("apps");
  const [formState, setFormState] = useState(() => getDefaultChannelFormState("webhook"));

  const isReadOnly = isReadOnlyStatus(agent?.status);
  const canManage = !policiesLoading && can("app.manage") && !isReadOnly;

  // The App row an endpoint still has to hang off until EVE-1011 deletes the
  // table. An agent normally has one; when it has several (the old shape was
  // one App per channel bundle) new endpoints join the first, and the existing
  // ones stay editable where they are.
  const carrierApp = apps?.find((app) => app.agent_id === agentId);

  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(`/agents/${agentId}`);
  }, [agent, agentId, canManage, policiesLoading, router]);

  const createEndpoint = useMutation({
    mutationFn: async () => {
      if (!canManage || !agent) throw new Error("Endpoint management is not available");
      // No carrier App yet: mint one named after the agent. It is never shown
      // as an App — the product surface for creating them is gone (EVE-998) —
      // it exists only to own the endpoint row for one more release.
      const app =
        carrierApp ??
        (await createApp({
          name: getDisplayName(agent),
          harness_id: agent.harness_id,
          agent_id: agent.id,
        }));
      return addChannel(app.id, {
        channel_type: formState.kind,
        channel_config: buildChannelConfig(formState),
        enabled: formState.enabled,
      });
    },
    onSuccess: (channel) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
      router.push(`/agents/${agentId}/endpoints/${channel.id}`);
    },
  });

  if (isLoading || appsLoading || policiesLoading)
    return <div className="container mx-auto p-6">Loading endpoint form...</div>;

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
          { label: agentName, href: `/agents/${agent.id}?tab=integrations` },
          { label: "New endpoint" },
        ]}
      />

      <PageMasthead
        icon={<Radio />}
        title="New endpoint"
        description="Expose this agent through a webhook, AG-UI, Slack, or one of the other transports."
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
            <Button
              type="button"
              variant="outline"
              onClick={() => router.push(`/agents/${agent.id}?tab=integrations`)}
            >
              Discard
            </Button>
          </>
        }
      />

      <form
        id="endpoint-edit-form"
        onSubmit={(e) => {
          e.preventDefault();
          createEndpoint.mutate();
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
            {carrierApp && <ChannelFormSummary app={carrierApp} state={formState} />}
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href={`/agents/${agent.id}?tab=integrations`}>Back to {agentName}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
