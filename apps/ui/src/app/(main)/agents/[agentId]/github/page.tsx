"use client";

// The agent's GitHub identity on a page of its own, so it has an address an
// external agent (or a person) can link to directly. It used to be a section of
// the Integrations tab; GitHub is who the agent is, not a way to reach it, so
// the Agent tab's settings list links here instead (D7,
// knowledge/integrations/github-model.md).

import { use } from "react";
import Link from "next/link";
import { GitPullRequest } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { useAgent } from "@/hooks/use-agents";
import { useAgentGitHub } from "@/hooks/use-agent-github";
import { useAgentTriggers } from "@/hooks/use-agent-triggers";
import { AgentGitHubCard } from "@/components/agents/agent-github-card";
import { triggerSummary } from "@/components/agents/trigger-list";
import { Badge } from "@/components/ui/badge";
import { ResourceNotFound } from "@/components/resource-not-found";
import {
  BackLink,
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageFooter,
  PageMain,
  PageMasthead,
  PageRail,
  RailSection,
} from "@/components/layout";
import { getDisplayName } from "@/lib/entity-lifecycle";

export default function AgentGitHubPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const ui = useUiTranslations();
  const { data: agent, isLoading, error } = useAgent(agentId);
  const { data: status } = useAgentGitHub(agentId);
  const { data: triggers = [] } = useAgentTriggers(agentId);

  if (isLoading) {
    return (
      <PageContainer>
        <p className="text-sm text-muted-foreground">{ui("Loading…")}</p>
      </PageContainer>
    );
  }
  if (error || !agent) {
    return (
      <ResourceNotFound
        title={ui("Agent not found")}
        description={ui("It may have been deleted, or you may not have access to it.")}
        backHref="/agents"
        backLabel={ui("Back to Agents")}
        resourceId={agentId}
      />
    );
  }

  const agentName = getDisplayName(agent);
  const githubTriggers = triggers.filter((trigger) => trigger.trigger_type === "github");
  const connected = !!status?.connected;
  const kind =
    status?.kind === "agent_app"
      ? ui("Own GitHub App")
      : status?.kind === "everruns_app"
        ? ui("Shared Everruns App")
        : ui("None yet");

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: ui("Agents"), href: "/agents" },
          { label: agentName, href: `/agents/${agentId}` },
          { label: ui("GitHub") },
        ]}
      />
      <PageMasthead
        icon={<GitPullRequest className="icon-sharp" />}
        title={ui("GitHub")}
        badges={
          <Badge variant={connected ? "success" : "secondary"}>
            {connected ? ui("Connected") : ui("Not connected")}
          </Badge>
        }
        description={ui(
          "Who {value0} is on GitHub when it reads code and comments on pull requests.",
          { value0: agentName },
        )}
        meta={status?.bot_login ? <span>{status.bot_login}</span> : undefined}
      />
      <PageColumns>
        <PageMain>
          <AgentGitHubCard
            agentId={agentId}
            avatar={agent.avatar}
            githubTriggerCount={githubTriggers.length}
          />
        </PageMain>
        <PageRail>
          <RailSection label={ui("Identity")}>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
              <dt className="text-muted-foreground">{ui("Kind")}</dt>
              <dd className="text-right">{kind}</dd>
              {status?.account && (
                <>
                  <dt className="text-muted-foreground">{ui("Account")}</dt>
                  <dd className="text-right">{status.account}</dd>
                </>
              )}
              {status?.repository_selection && (
                <>
                  <dt className="text-muted-foreground">{ui("Repositories")}</dt>
                  <dd className="text-right">
                    {status.repository_selection === "all" ? ui("All") : ui("Selected")}
                  </dd>
                </>
              )}
            </dl>
          </RailSection>
          <RailSection label={ui("Used by")}>
            <ul className="flex flex-col gap-1.5 text-sm">
              {githubTriggers.map((trigger) => (
                <li key={trigger.id}>
                  <Link
                    href={`/agents/${agentId}/triggers/${trigger.id}`}
                    className="underline underline-offset-4"
                  >
                    {triggerSummary(trigger).title}
                  </Link>
                  <span className="text-muted-foreground"> · {ui("trigger")}</span>
                </li>
              ))}
              <li className="text-muted-foreground">
                {ui("The agent's GitHub tools, while the GitHub capability is on")}
              </li>
            </ul>
          </RailSection>
        </PageRail>
      </PageColumns>
      <PageFooter>
        <BackLink href={`/agents/${agentId}`}>
          {ui("Back to")} {agentName}
        </BackLink>
      </PageFooter>
    </PageContainer>
  );
}
