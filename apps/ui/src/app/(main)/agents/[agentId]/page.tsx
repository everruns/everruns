"use client";

import { use, useMemo, useCallback, useState } from "react";
import {
  useAgent,
  useSessions,
  useCreateSession,
  useCapabilities,
  useModels,
  useExportAgent,
  useCopyAgent,
  useAgentStats,
  usePageTitle,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { SessionCard } from "@/components/session/session-card";
import { AgentPreview } from "@/components/agents/agent-preview";
import { AgentVersionHistory } from "@/components/agents/agent-version-history";
import { IntegrationGuide } from "@/components/integration/integration-guide";
import {
  Plus,
  Pencil,
  Download,
  Copy,
  Zap,
  Eye,
  LayoutDashboard,
  BarChart3,
  GitBranch,
  Rocket,
  Telescope,
  Terminal,
  Boxes,
} from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import { ResourceStatsPanel } from "@/components/stats/resource-stats-panel";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
  type SectionTabItem,
} from "@/components/layout";
import type { Capability, ModelWithProvider, TokenUsage } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
} from "@/lib/entity-lifecycle";
import { formatTokens, pluralize } from "@/lib/formatting";
import { normalizeTags } from "@/lib/tags";
import { useFeatureFlag } from "@/providers/feature-flags-provider";

// Helper function to calculate total tokens
function totalTokens(usage: TokenUsage): number {
  return usage.input_tokens + usage.output_tokens;
}

export default function AgentDetailPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const { locale } = useLocale();
  const router = useRouter();
  const [activeTab, setActiveTab] = useState("overview");
  const agentVersionsEnabled = useFeatureFlag("agent_versions");
  const observersEnabled = useFeatureFlag("observers");
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  usePageTitle(agent ? getDisplayName(agent) : null, "Agent");
  // Fetch only top 10 sessions for the overview
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(agentId, {
    limit: 10,
  });
  const sessions = sessionsResponse?.data ?? [];
  const totalSessions = sessionsResponse?.total ?? 0;
  const hasMoreSessions = totalSessions > 10;
  const { data: allCapabilities } = useCapabilities();
  const { data: models } = useModels();
  const { data: stats, isLoading: statsLoading, error: statsError } = useAgentStats(agentId);
  const createSession = useCreateSession();
  const exportAgent = useExportAgent();
  const copyAgent = useCopyAgent();

  // Create a map of model_id -> model for quick lookups
  const modelMap = useMemo(() => {
    if (!models) return new Map<string, ModelWithProvider>();
    return new Map(models.map((m) => [m.id, m]));
  }, [models]);

  // Get the agent's default model
  const defaultModel = agent?.default_model_id ? modelMap.get(agent.default_model_id) : undefined;
  const agentTags = normalizeTags(agent?.tags);

  const handleNewSession = async () => {
    try {
      // Agent-first: omit the harness so the server derives it from the agent's
      // own harness (falling back to the org default only when the agent has none).
      const session = await createSession.mutateAsync({
        request: { agent_id: agentId },
      });
      router.push(`/sessions/${session.id}`);
    } catch (error) {
      console.error("Failed to create session:", error);
    }
  };

  const handleExport = useCallback(async () => {
    if (!agent) return;
    try {
      const markdown = await exportAgent.mutateAsync(agentId);
      // Create downloadable file
      const blob = new Blob([markdown], { type: "text/markdown" });
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = `${agent.name}.md`;
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      URL.revokeObjectURL(url);
    } catch (error) {
      console.error("Failed to export agent:", error);
    }
  }, [agent, agentId, exportAgent]);

  const handleCopy = useCallback(async () => {
    try {
      const copied = await copyAgent.mutateAsync(agentId);
      router.push(`/agents/${copied.id}`);
    } catch (error) {
      console.error("Failed to copy agent:", error);
    }
  }, [agentId, copyAgent, router]);

  const getCapabilityInfo = (capabilityId: string): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  // Capabilities are now part of the agent resource
  const agentCapabilities = agent?.capabilities ?? [];
  const agentSessionCount = agent?.session_count ?? totalSessions;
  const agentAppCount = agent?.app_count ?? 0;

  if (agentLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
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

  const defaultModelName = defaultModel?.display_name ?? defaultModel?.id;

  const tabItems: SectionTabItem[] = [
    { value: "overview", label: "Overview", icon: <LayoutDashboard className="size-4" /> },
    { value: "preview", label: "Preview", icon: <Eye className="size-4" /> },
    { value: "integrate", label: "Integrate", icon: <Terminal className="size-4" /> },
    { value: "stats", label: "Stats", icon: <BarChart3 className="size-4" /> },
    ...(agentVersionsEnabled
      ? [{ value: "versions", label: "Versions", icon: <GitBranch className="size-4" /> }]
      : []),
  ];

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: "Agents", href: "/agents" }, { label: getDisplayName(agent) }]}
      />

      <PageMasthead
        icon={<Boxes />}
        title={
          <span className={getEntityNameClassName(agent.status)}>{getDisplayName(agent)}</span>
        }
        badges={
          <>
            <CopyButton value={agent.id} />
            <Badge variant={getEntityStatusBadgeVariant(agent.status)}>{agent.status}</Badge>
          </>
        }
        description={agent.description || undefined}
        meta={
          <>
            <span>
              Identity <span className="font-mono text-primary">{agent.name}</span>
            </span>
            {defaultModelName && (
              <span>
                Model <span className="text-primary">{defaultModelName}</span>
              </span>
            )}
            <span>
              Created{" "}
              <span className="text-foreground">
                {new Date(agent.created_at).toLocaleDateString()}
              </span>
            </span>
            <span>
              <span className="text-foreground">{agentSessionCount}</span>{" "}
              {pluralize(agentSessionCount, "session")}
            </span>
          </>
        }
        actions={
          <>
            <Button variant="outline" onClick={handleCopy} disabled={copyAgent.isPending}>
              <Copy className="size-4" />
              {copyAgent.isPending ? "Copying..." : "Copy"}
            </Button>
            <Button variant="outline" onClick={handleExport} disabled={exportAgent.isPending}>
              <Download className="size-4" />
              {exportAgent.isPending ? "Exporting..." : "Export"}
            </Button>
            {agent.status === "active" && (
              <Link href={`/agents/${agentId}/edit`}>
                <Button variant="outline">
                  <Pencil className="size-4" />
                  Edit
                </Button>
              </Link>
            )}
            {agent.status === "active" && (
              <Link href={{ pathname: "/apps/new", query: { agent_id: agentId } }}>
                <Button variant="outline">
                  <Rocket className="size-4" />
                  Create app
                </Button>
              </Link>
            )}
            {observersEnabled && agent.status === "active" && (
              <Link href={{ pathname: "/observers/new", query: { agent_id: agentId } }}>
                <Button variant="outline">
                  <Telescope className="size-4" />
                  Observe this agent
                </Button>
              </Link>
            )}
            <Button
              variant="accent"
              onClick={handleNewSession}
              disabled={createSession.isPending || agent.status !== "active"}
            >
              <Plus className="size-4" />
              {createSession.isPending ? "Creating..." : "New session"}
            </Button>
          </>
        }
      />

      <PageControlStrip>
        <SectionTabs value={activeTab} onValueChange={setActiveTab} items={tabItems} />
      </PageControlStrip>

      {activeTab === "overview" && (
        <PageColumns>
          <PageMain>
            <Card>
              <CardHeader>
                <CardTitle>System Prompt</CardTitle>
              </CardHeader>
              <CardContent>
                <MarkdownDisplay content={agent.system_prompt} />
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between">
                <CardTitle>Sessions</CardTitle>
                {hasMoreSessions && (
                  <Link
                    href={`/agents/${agentId}/sessions`}
                    className="text-sm text-muted-foreground hover:text-foreground"
                  >
                    View all {totalSessions} sessions →
                  </Link>
                )}
              </CardHeader>
              <CardContent>
                {sessionsLoading ? (
                  <div className="space-y-2">
                    <Skeleton className="h-12 w-full" />
                    <Skeleton className="h-12 w-full" />
                  </div>
                ) : sessions.length === 0 ? (
                  <p className="text-center py-8 text-muted-foreground">
                    No sessions yet. Start a new session to begin chatting.
                  </p>
                ) : (
                  <div className="space-y-2">
                    {sessions.map((session) => (
                      <SessionCard
                        key={session.id}
                        session={session}
                        model={session.model_id ? modelMap.get(session.model_id) : undefined}
                      />
                    ))}
                    {hasMoreSessions && (
                      <Link
                        href={`/agents/${agentId}/sessions`}
                        className="flex items-center justify-center p-3 border border-dashed hover:bg-muted transition-colors text-muted-foreground"
                      >
                        View all {totalSessions} sessions
                      </Link>
                    )}
                  </div>
                )}
              </CardContent>
            </Card>
          </PageMain>

          <PageRail>
            <Card>
              <CardHeader>
                <CardTitle>Capabilities</CardTitle>
              </CardHeader>
              <CardContent>
                {agentCapabilities.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    No capabilities enabled.{" "}
                    <Link href={`/agents/${agentId}/edit`} className="text-primary hover:underline">
                      Add some
                    </Link>
                  </p>
                ) : (
                  <div className="space-y-2">
                    {agentCapabilities.map((capConfig) => {
                      const cap = getCapabilityInfo(capConfig.ref);
                      if (!cap) return null;
                      const IconComponent = getCapabilityIcon(cap.icon);

                      return (
                        <div
                          key={capConfig.ref}
                          className="flex items-center gap-2 p-2 border bg-muted/50"
                        >
                          <IconComponent className="w-4 h-4" />
                          <div className="flex-1">
                            <p className="text-sm font-medium">
                              {localizedCapabilityName(cap, locale)}
                            </p>
                            <p className="text-xs text-muted-foreground">
                              {localizedCapabilityDescription(cap, locale)}
                            </p>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Configuration</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                {defaultModel && (
                  <div>
                    <p className="text-sm font-medium mb-2">Default Model</p>
                    <div className="flex items-center gap-2">
                      <ProviderIcon providerType={defaultModel.provider_type} size="sm" />
                      <span className="text-sm">{defaultModel.display_name}</span>
                    </div>
                  </div>
                )}

                {agent.description && (
                  <div>
                    <p className="text-sm font-medium">Description</p>
                    <div className="text-sm text-muted-foreground">
                      <InlineStreamdownMessage>{agent.description}</InlineStreamdownMessage>
                    </div>
                  </div>
                )}

                {agentTags.length > 0 && (
                  <div>
                    <p className="text-sm font-medium mb-2">Tags</p>
                    <div className="flex flex-wrap gap-1">
                      {agentTags.map((tag) => (
                        <Badge key={tag} variant="outline">
                          {tag}
                        </Badge>
                      ))}
                    </div>
                  </div>
                )}

                <div>
                  <p className="text-sm font-medium mb-2">Usage</p>
                  <div className="grid grid-cols-2 gap-2">
                    <Link
                      href={`/agents/${agentId}/sessions`}
                      className="border bg-muted/50 p-2 hover:bg-muted"
                    >
                      <p className="text-sm font-medium">{agentSessionCount}</p>
                      <p className="text-xs text-muted-foreground">
                        {pluralize(agentSessionCount, "session")}
                      </p>
                    </Link>
                    <div className="border bg-muted/50 p-2">
                      <p className="text-sm font-medium">{agentAppCount}</p>
                      <p className="text-xs text-muted-foreground">
                        {pluralize(agentAppCount, "app")}
                      </p>
                    </div>
                  </div>
                </div>

                {agent.usage && (
                  <div>
                    <p className="text-sm font-medium mb-2">Token Usage</p>
                    <div className="flex items-center gap-2 p-2 border bg-muted/50">
                      <Zap className="w-4 h-4 text-yellow-500" />
                      <div className="flex-1">
                        <p className="text-sm font-medium">
                          {formatTokens(totalTokens(agent.usage))} total
                        </p>
                        <p className="text-xs text-muted-foreground">
                          {formatTokens(agent.usage.input_tokens)} input /{" "}
                          {formatTokens(agent.usage.output_tokens)} output
                          {agent.usage.cache_read_tokens &&
                            ` / ${formatTokens(agent.usage.cache_read_tokens)} cached`}
                        </p>
                      </div>
                    </div>
                  </div>
                )}

                <div>
                  <p className="text-sm font-medium">Created</p>
                  <p className="text-sm text-muted-foreground">
                    {new Date(agent.created_at).toLocaleString()}
                  </p>
                </div>

                <div>
                  <p className="text-sm font-medium">Updated</p>
                  <p className="text-sm text-muted-foreground">
                    {new Date(agent.updated_at).toLocaleString()}
                  </p>
                </div>
              </CardContent>
            </Card>
          </PageRail>
        </PageColumns>
      )}

      {activeTab === "preview" && (
        <AgentPreview
          systemPrompt={agent.system_prompt}
          capabilities={agentCapabilities.map((cap) => ({
            ref: cap.ref,
            config: cap.config,
          }))}
          initialFiles={agent.initial_files}
          tools={agent.tools ?? []}
        />
      )}

      {activeTab === "integrate" && (
        <IntegrationGuide kind="agent" id={agent.id} name={getDisplayName(agent)} />
      )}

      {activeTab === "stats" && (
        <ResourceStatsPanel stats={stats} isLoading={statsLoading} error={statsError} />
      )}

      {agentVersionsEnabled && activeTab === "versions" && <AgentVersionHistory agent={agent} />}

      <PageFooter>
        <BackLink href="/agents">Back to Agents</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
