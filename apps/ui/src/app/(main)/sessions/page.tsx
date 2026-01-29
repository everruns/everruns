"use client";

import { useState, useMemo } from "react";
import { useAgents, useSessions, useCreateSession, useLlmModels } from "@/hooks";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SessionCard } from "@/components/session/session-card";
import { AgentSelect } from "@/components/agent/agent-select";
import { AgentFilterMenu } from "@/components/agent/agent-filter-menu";
import { Plus, ChevronLeft, ChevronRight } from "lucide-react";
import type { LlmModelWithProvider, Agent } from "@/lib/api/types";

const PAGE_SIZE = 20;

export default function SessionsPage() {
  const router = useRouter();
  const [page, setPage] = useState(0);
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [newSessionDialogOpen, setNewSessionDialogOpen] = useState(false);
  const [newSessionAgentId, setNewSessionAgentId] = useState<string>("");
  const offset = page * PAGE_SIZE;

  const { data: agents, isLoading: agentsLoading } = useAgents();
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(
    selectedAgentId || undefined, // Pass undefined to get all sessions
    { offset, limit: PAGE_SIZE },
  );
  const { data: llmModels } = useLlmModels();
  const createSession = useCreateSession();

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

  const handleOpenNewSessionDialog = () => {
    // Pre-select the filtered agent if one is selected, otherwise first agent
    setNewSessionAgentId(selectedAgentId || agents?.[0]?.id || "");
    setNewSessionDialogOpen(true);
  };

  const handleCreateSession = async () => {
    if (!newSessionAgentId) {
      console.error("No agent selected");
      return;
    }
    try {
      const session = await createSession.mutateAsync({
        request: { agent_id: newSessionAgentId },
      });
      setNewSessionDialogOpen(false);
      // Use org-level session URL
      router.push(`/sessions/${session.id}`);
    } catch (error) {
      console.error("Failed to create session:", error);
    }
  };

  const handlePreviousPage = () => {
    setPage((p) => Math.max(0, p - 1));
  };

  const handleNextPage = () => {
    setPage((p) => Math.min(totalPages - 1, p + 1));
  };

  const handleAgentFilterChange = (agentId: string) => {
    setSelectedAgentId(agentId);
    setPage(0); // Reset pagination when filter changes
  };

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Sessions</h1>
        <Button variant="accent" onClick={handleOpenNewSessionDialog} disabled={!agents?.length}>
          <Plus className="w-4 h-4 mr-2" />
          New Session
        </Button>
      </div>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0">
          <div>
            <CardTitle>
              {selectedAgentId ? agentMap.get(selectedAgentId)?.name || "Agent" : "All Agents"}
            </CardTitle>
            <p className="text-sm text-muted-foreground mt-1">
              {totalSessions} session{totalSessions !== 1 ? "s" : ""}
            </p>
          </div>
          <AgentFilterMenu value={selectedAgentId} onValueChange={handleAgentFilterChange} />
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
                  <SessionCard
                    key={session.id}
                    session={session}
                    agentName={agent?.name}
                    model={session.model_id ? modelMap.get(session.model_id) : undefined}
                  />
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

      {/* New Session Dialog */}
      <Dialog open={newSessionDialogOpen} onOpenChange={setNewSessionDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New Session</DialogTitle>
            <DialogDescription>Select an agent to start a new conversation.</DialogDescription>
          </DialogHeader>
          <div className="py-4">
            <AgentSelect
              value={newSessionAgentId}
              onValueChange={setNewSessionAgentId}
              placeholder="Select an agent"
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setNewSessionDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleCreateSession}
              disabled={!newSessionAgentId || createSession.isPending}
            >
              {createSession.isPending ? "Creating..." : "Create Session"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
