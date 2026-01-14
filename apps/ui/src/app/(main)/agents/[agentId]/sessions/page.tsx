"use client";

import { use, useState, useMemo } from "react";
import { useAgent, useSessions, useCreateSession, useLlmModels, useDeleteSession } from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { SessionCard } from "@/components/session/session-card";
import {
  ArrowLeft,
  Plus,
  ChevronLeft,
  ChevronRight,
  Trash2,
} from "lucide-react";
import type { LlmModelWithProvider } from "@/lib/api/types";

const PAGE_SIZE = 20;

export default function SessionsListPage({
  params,
}: {
  params: Promise<{ agentId: string }>;
}) {
  const { agentId } = use(params);
  const router = useRouter();
  const [page, setPage] = useState(0);
  const offset = page * PAGE_SIZE;

  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(
    agentId,
    { offset, limit: PAGE_SIZE }
  );
  const { data: llmModels } = useLlmModels();
  const createSession = useCreateSession();
  const deleteSession = useDeleteSession();

  const sessions = sessionsResponse?.data ?? [];
  const totalSessions = sessionsResponse?.total ?? 0;
  const totalPages = Math.ceil(totalSessions / PAGE_SIZE);

  // Create a map of model_id -> model for quick lookups
  const modelMap = useMemo(() => {
    if (!llmModels) return new Map<string, LlmModelWithProvider>();
    return new Map(llmModels.map((m) => [m.id, m]));
  }, [llmModels]);

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

  const handleDeleteSession = async (sessionId: string, sessionTitle: string) => {
    const confirmed = window.confirm(
      `Are you sure you want to delete "${sessionTitle}"? This action cannot be undone.`
    );
    if (!confirmed) return;

    try {
      await deleteSession.mutateAsync({ agentId, sessionId });
    } catch (error) {
      console.error("Failed to delete session:", error);
    }
  };

  const handlePreviousPage = () => {
    setPage((p) => Math.max(0, p - 1));
  };

  const handleNextPage = () => {
    setPage((p) => Math.min(totalPages - 1, p + 1));
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
        href={`/agents/${agentId}`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to {agent.name}
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">{agent.name} - Sessions</h1>
          <p className="text-muted-foreground">
            {totalSessions} session{totalSessions !== 1 ? "s" : ""} total
          </p>
        </div>
        <Button onClick={handleNewSession} disabled={createSession.isPending}>
          <Plus className="w-4 h-4 mr-2" />
          {createSession.isPending ? "Creating..." : "New Session"}
        </Button>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>All Sessions</CardTitle>
        </CardHeader>
        <CardContent>
          {sessionsLoading ? (
            <div className="space-y-2">
              {Array.from({ length: 5 }).map((_, i) => (
                <Skeleton key={i} className="h-16 w-full" />
              ))}
            </div>
          ) : sessions.length === 0 ? (
            <p className="text-center py-8 text-muted-foreground">
              No sessions yet. Start a new session to begin chatting.
            </p>
          ) : (
            <div className="space-y-2">
              {sessions.map((session) => (
                <div
                  key={session.id}
                  className="flex items-center gap-2"
                >
                  <div className="flex-1">
                    <SessionCard
                      session={session}
                      agentId={agentId}
                      model={session.model_id ? modelMap.get(session.model_id) : undefined}
                    />
                  </div>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8 text-muted-foreground hover:text-destructive flex-shrink-0"
                    onClick={() =>
                      handleDeleteSession(
                        session.id,
                        session.title || `Session ${session.id.slice(0, 8)}`
                      )
                    }
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              ))}
            </div>
          )}

          {/* Pagination controls */}
          {totalPages > 1 && (
            <div className="flex items-center justify-between mt-4 pt-4 border-t">
              <p className="text-sm text-muted-foreground">
                Showing {offset + 1}-{Math.min(offset + PAGE_SIZE, totalSessions)} of{" "}
                {totalSessions} sessions
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handlePreviousPage}
                  disabled={page === 0}
                >
                  <ChevronLeft className="h-4 w-4 mr-1" />
                  Previous
                </Button>
                <span className="text-sm text-muted-foreground">
                  Page {page + 1} of {totalPages}
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleNextPage}
                  disabled={page >= totalPages - 1}
                >
                  Next
                  <ChevronRight className="h-4 w-4 ml-1" />
                </Button>
              </div>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
