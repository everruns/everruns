"use client";

import { use } from "react";
import { useAgent, useSessions, useCreateSession } from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { ArrowLeft, Plus, MessageSquare } from "lucide-react";

export default function AgentDetailPage({
  params,
}: {
  params: Promise<{ agentId: string }>;
}) {
  const { agentId } = use(params);
  const router = useRouter();
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { data: sessions, isLoading: sessionsLoading } = useSessions(agentId);
  const createSession = useCreateSession();

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
        <Button onClick={handleNewSession} disabled={createSession.isPending}>
          <Plus className="w-4 h-4 mr-2" />
          {createSession.isPending ? "Creating..." : "New Session"}
        </Button>
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
                      {session.finished_at && (
                        <Badge variant="outline">Completed</Badge>
                      )}
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
              <CardTitle>Configuration</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
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
