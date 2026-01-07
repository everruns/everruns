"use client";

import { use, useMemo, useCallback } from "react";
import { useAgent, useSessions, useCreateSession, useCapabilities, useLlmModels, useExportAgent } from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { ProviderIcon } from "@/components/providers/provider-icon";
import {
  ArrowLeft,
  Plus,
  MessageSquare,
  Pencil,
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  Download,
  LucideIcon,
} from "lucide-react";
import type { Capability, LlmModelWithProvider } from "@/lib/api/types";

const iconMap: Record<string, LucideIcon> = {
  "circle-off": CircleOff,
  clock: Clock,
  search: Search,
  box: Box,
  folder: Folder,
};

export default function AgentDetailPage({
  params,
}: {
  params: Promise<{ agentId: string }>;
}) {
  const { agentId } = use(params);
  const router = useRouter();
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { data: sessions, isLoading: sessionsLoading } = useSessions(agentId);
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
        agentId,
        request: {},
      });
      router.push(`/agents/${agentId}/sessions/${session.id}`);
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
            <Badge
              variant={agent.status === "active" ? "default" : "secondary"}
            >
              {agent.status}
            </Badge>
          </h1>
          <p className="text-muted-foreground font-mono text-sm">
            ID: {agent.id.slice(0, 8)}...
          </p>
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
          <Button onClick={handleNewSession} disabled={createSession.isPending}>
            <Plus className="w-4 h-4 mr-2" />
            {createSession.isPending ? "Creating..." : "New Session"}
          </Button>
        </div>
      </div>

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
            <CardHeader>
              <CardTitle>Sessions</CardTitle>
            </CardHeader>
            <CardContent>
              {sessionsLoading ? (
                <div className="space-y-2">
                  <Skeleton className="h-12 w-full" />
                  <Skeleton className="h-12 w-full" />
                </div>
              ) : sessions?.length === 0 ? (
                <p className="text-center py-8 text-muted-foreground">
                  No sessions yet. Start a new session to begin chatting.
                </p>
              ) : (
                <div className="space-y-2">
                  {sessions?.map((session) => (
                    <Link
                      key={session.id}
                      href={`/agents/${agentId}/sessions/${session.id}`}
                      className="flex items-center justify-between p-3 rounded-md border hover:bg-muted transition-colors"
                    >
                      <div className="flex items-center gap-3">
                        <MessageSquare className="w-4 h-4 text-muted-foreground" />
                        <div>
                          <p className="font-medium">
                            {session.title || `Session ${session.id.slice(0, 8)}`}
                          </p>
                          <p className="text-xs text-muted-foreground">
                            {new Date(session.created_at).toLocaleString()}
                          </p>
                        </div>
                      </div>
                      <div className="flex items-center gap-2">
                        {session.model_id && modelMap.get(session.model_id) && (
                          <Badge variant="outline" className="gap-1 text-xs">
                            <ProviderIcon
                              providerType={modelMap.get(session.model_id)!.provider_type}
                              size="sm"
                              showBackground={false}
                            />
                            {modelMap.get(session.model_id)!.display_name}
                          </Badge>
                        )}
                        {session.finished_at && (
                          <Badge variant="outline">Completed</Badge>
                        )}
                      </div>
                    </Link>
                  ))}
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
                  {agentCapabilities.map((capId) => {
                    const cap = getCapabilityInfo(capId);
                    if (!cap) return null;
                    const IconComponent = cap.icon
                      ? iconMap[cap.icon] || CircleOff
                      : CircleOff;

                    return (
                      <div
                        key={capId}
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
                  <p className="text-sm text-muted-foreground">
                    {agent.description}
                  </p>
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
    </div>
  );
}
