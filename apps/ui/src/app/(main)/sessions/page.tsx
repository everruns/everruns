"use client";

import { useState, useMemo } from "react";
import { useAgents, useSessions, useCreateSession, useLlmModels, useDeleteSession } from "@/hooks";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { SessionCard } from "@/components/session/session-card";
import {
  Plus,
  ChevronLeft,
  ChevronRight,
  Trash2,
} from "lucide-react";
import type { LlmModelWithProvider, Agent } from "@/lib/api/types";

const PAGE_SIZE = 20;

export default function SessionsPage() {
  const router = useRouter();
  const [page, setPage] = useState(0);
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const offset = page * PAGE_SIZE;

  const { data: agents, isLoading: agentsLoading } = useAgents();
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(
    selectedAgentId || undefined, // Pass undefined to get all sessions
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

  // Create a map of agent_id -> agent for quick lookups
  const agentMap = useMemo(() => {
    if (!agents) return new Map<string, Agent>();
    return new Map(agents.map((a) => [a.id, a]));
  }, [agents]);

  const handleNewSession = async () => {
    // If an agent is selected, create session for that agent
    // Otherwise, show the first available agent
    const targetAgentId = selectedAgentId || agents?.[0]?.id;
    if (!targetAgentId) {
      console.error("No agent available to create session");
      return;
    }
    try {
      const session = await createSession.mutateAsync({
        request: { agent_id: targetAgentId },
      });
      // Use org-level session URL
      router.push(`/sessions/${session.id}`);
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
      await deleteSession.mutateAsync({ sessionId });
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

  const handleAgentFilterChange = (value: string) => {
    setSelectedAgentId(value === "all" ? "" : value);
    setPage(0); // Reset pagination when filter changes
  };

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">Sessions</h1>
          <p className="text-muted-foreground">
            {totalSessions} session{totalSessions !== 1 ? "s" : ""} total
          </p>
        </div>
        <div className="flex items-center gap-4">
          <Select
            value={selectedAgentId || "all"}
            onValueChange={handleAgentFilterChange}
          >
            <SelectTrigger className="w-[200px]">
              <SelectValue placeholder="Filter by agent" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All agents</SelectItem>
              {agents?.map((agent) => (
                <SelectItem key={agent.id} value={agent.id}>
                  {agent.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            onClick={handleNewSession}
            disabled={createSession.isPending || !agents?.length}
          >
            <Plus className="w-4 h-4 mr-2" />
            {createSession.isPending ? "Creating..." : "New Session"}
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>
            {selectedAgentId
              ? `Sessions for ${agentMap.get(selectedAgentId)?.name || "Agent"}`
              : "All Sessions"}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {sessionsLoading || agentsLoading ? (
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
              {sessions.map((session) => {
                const agent = agentMap.get(session.agent_id);
                return (
                  <div key={session.id} className="flex items-center gap-2">
                    <div className="flex-1">
                      <SessionCard
                        session={session}
                        agentName={!selectedAgentId ? agent?.name : undefined}
                        model={session.model_id ? modelMap.get(session.model_id) : undefined}
                        useOrgLevelUrl={true}
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
                );
              })}
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
