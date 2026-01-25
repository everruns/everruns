"use client";

import { use, useMemo, useCallback, useState } from "react";
import { useAgent, useSessions, useCreateSession, useCapabilities, useLlmModels, useExportAgent } from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { InlineMarkdown } from "@/components/ui/markdown";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { SessionCard } from "@/components/session/session-card";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { AgentPreview } from "@/components/agents/agent-preview";
import {
  ArrowLeft,
  Plus,
  Pencil,
  Download,
  Zap,
  Eye,
  LayoutDashboard,
} from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import type { Capability, LlmModelWithProvider, TokenUsage } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";

// Helper function to format token counts in a compact way
function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(1)}M`;
  }
  if (tokens >= 1000) {
    return `${(tokens / 1000).toFixed(1)}K`;
  }
  return tokens.toString();
}

// Helper function to calculate total tokens
function totalTokens(usage: TokenUsage): number {
  return usage.input_tokens + usage.output_tokens;
}

export default function AgentDetailPage({
  params,
}: {
  params: Promise<{ agentId: string }>;
}) {
  const { agentId } = use(params);
  const router = useRouter();
  const [activeTab, setActiveTab] = useState("overview");
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  // Fetch only top 10 sessions for the overview
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(agentId, { limit: 10 });
  const sessions = sessionsResponse?.data ?? [];
  const totalSessions = sessionsResponse?.total ?? 0;
  const hasMoreSessions = totalSessions > 10;
  const { data: allCapabilities } = useCapabilities();
  const { data: llmModels } = useLlmModels();
  const createSession = useCreateSession();
  const exportAgent = useExportAgent();

  // Create a map of model_id -> model for quick lookups
  const modelMap = useMemo(() => {
    if (!llmModels) return new Map<string, LlmModelWithProvider>();
    return new Map(llmModels.map((m) => [m.id, m]));
  }, [llmModels]);

  // Get the agent's default model
  const defaultModel = agent?.default_model_id ? modelMap.get(agent.default_model_id) : undefined;

  const handleNewSession = async () => {
    try {
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
      link.download = `${agent.name.toLowerCase().replace(/\s+/g, "-")}.md`;
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      URL.revokeObjectURL(url);
    } catch (error) {
      console.error("Failed to export agent:", error);
    }
  }, [agent, agentId, exportAgent]);

  const getCapabilityInfo = (capabilityId: string): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  // Capabilities are now part of the agent resource
  const agentCapabilities = agent?.capabilities ?? [];

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
      <div className="container mx-auto p-6">
        <div className="text-red-500">Agent not found</div>
        <Link href="/agents" className="text-blue-500 hover:underline">
          Back to agents
        </Link>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/agents"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Agents
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            {agent.name}
            <CopyButton value={agent.id} />
            <Badge
              variant={agent.status === "active" ? "default" : "secondary"}
            >
              {agent.status}
            </Badge>
          </h1>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={handleExport}
            disabled={exportAgent.isPending}
          >
            <Download className="w-4 h-4 mr-2" />
            {exportAgent.isPending ? "Exporting..." : "Export"}
          </Button>
          <Link href={`/agents/${agentId}/edit`}>
            <Button variant="outline">
              <Pencil className="w-4 h-4 mr-2" />
              Edit
            </Button>
          </Link>
          <Button variant="accent" onClick={handleNewSession} disabled={createSession.isPending}>
            <Plus className="w-4 h-4 mr-2" />
            {createSession.isPending ? "Creating..." : "New Session"}
          </Button>
        </div>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <TabsList className="mb-6">
          <TabsTrigger value="overview">
            <LayoutDashboard className="w-4 h-4 mr-2" />
            Overview
          </TabsTrigger>
          <TabsTrigger value="preview">
            <Eye className="w-4 h-4 mr-2" />
            Preview
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview">
          <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
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
                      className="flex items-center justify-center p-3 rounded-md border border-dashed hover:bg-muted transition-colors text-muted-foreground"
                    >
                      View all {totalSessions} sessions
                    </Link>
                  )}
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        <div className="space-y-6">
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
                        className="flex items-center gap-2 p-2 rounded-md border bg-muted/50"
                      >
                        <IconComponent className="w-4 h-4" />
                        <div className="flex-1">
                          <p className="text-sm font-medium">{cap.name}</p>
                          <p className="text-xs text-muted-foreground">
                            {cap.description}
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
                    <ProviderIcon
                      providerType={defaultModel.provider_type}
                      size="sm"
                    />
                    <span className="text-sm">{defaultModel.display_name}</span>
                  </div>
                </div>
              )}

              {agent.description && (
                <div>
                  <p className="text-sm font-medium">Description</p>
                  <div className="text-sm text-muted-foreground">
                    <InlineMarkdown content={agent.description} />
                  </div>
                </div>
              )}

              {agent.tags.length > 0 && (
                <div>
                  <p className="text-sm font-medium mb-2">Tags</p>
                  <div className="flex flex-wrap gap-1">
                    {agent.tags.map((tag) => (
                      <Badge key={tag} variant="outline">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}

              {agent.usage && (
                <div>
                  <p className="text-sm font-medium mb-2">Token Usage</p>
                  <div className="flex items-center gap-2 p-2 rounded-md border bg-muted/50">
                    <Zap className="w-4 h-4 text-yellow-500" />
                    <div className="flex-1">
                      <p className="text-sm font-medium">{formatTokens(totalTokens(agent.usage))} total</p>
                      <p className="text-xs text-muted-foreground">
                        {formatTokens(agent.usage.input_tokens)} input / {formatTokens(agent.usage.output_tokens)} output
                        {agent.usage.cache_read_tokens && ` / ${formatTokens(agent.usage.cache_read_tokens)} cached`}
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
        </div>
          </div>
        </TabsContent>

        <TabsContent value="preview">
          <AgentPreview
            systemPrompt={agent.system_prompt}
            capabilities={agentCapabilities.map((cap) => ({
              ref: cap.ref,
              config: cap.config,
            }))}
          />
        </TabsContent>
      </Tabs>
    </div>
  );
}
