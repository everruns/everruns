"use client";

import { useState, useMemo } from "react";
import {
  useAgents,
  useHarnesses,
  useSessions,
  useCreateSession,
  useLlmModels,
  usePinSession,
  useUnpinSession,
} from "@/hooks";
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
import { HarnessSelect } from "@/components/harness/harness-select";
import { AgentFilterMenu } from "@/components/agent/agent-filter-menu";
import { Label } from "@/components/ui/label";
import { Plus, ChevronLeft, ChevronRight } from "lucide-react";
import type { LlmModelWithProvider, Agent } from "@/lib/api/types";

const PAGE_SIZE = 20;

export default function SessionsPage() {
  const router = useRouter();
  const [page, setPage] = useState(0);
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [newSessionDialogOpen, setNewSessionDialogOpen] = useState(false);
  const [newSessionHarnessId, setNewSessionHarnessId] = useState<string>("");
  const [newSessionAgentId, setNewSessionAgentId] = useState<string>("");
  const offset = page * PAGE_SIZE;

  const { data: agents, isLoading: agentsLoading } = useAgents();
  const { data: harnesses } = useHarnesses();
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(
    selectedAgentId || undefined, // Pass undefined to get all sessions
    { offset, limit: PAGE_SIZE },
  );
  const { data: llmModels } = useLlmModels();
  const createSession = useCreateSession();
  const pinSessionMutation = usePinSession();
  const unpinSessionMutation = useUnpinSession();

  // Sort pinned sessions first, preserving server order within each group
  const sessions = useMemo(() => {
    const data = sessionsResponse?.data ?? [];
    return [...data].sort((a, b) => {
      const aPinned = a.is_pinned === true ? 1 : 0;
      const bPinned = b.is_pinned === true ? 1 : 0;
      return bPinned - aPinned;
    });
  }, [sessionsResponse?.data]);
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
    // Pre-select the first harness
    setNewSessionHarnessId(harnesses?.[0]?.id || "");
    // Pre-select the filtered agent if one is selected, otherwise leave empty (optional)
    setNewSessionAgentId(selectedAgentId || "");
    setNewSessionDialogOpen(true);
  };

  const handleCreateSession = async () => {
    if (!newSessionHarnessId) {
      console.error("No harness selected");
      return;
    }
    try {
      const session = await createSession.mutateAsync({
        request: {
          harness_id: newSessionHarnessId,
          agent_id: newSessionAgentId || undefined,
        },
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

  const handleTogglePin = (sessionId: string, pinned: boolean) => {
    if (pinned) {
      pinSessionMutation.mutate({ sessionId });
    } else {
      unpinSessionMutation.mutate({ sessionId });
    }
  };

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Sessions</h1>
        <Button variant="accent" onClick={handleOpenNewSessionDialog} disabled={!harnesses?.length}>
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
                const agent = session.agent_id ? agentMap.get(session.agent_id) : undefined;
                return (
                  <SessionCard
                    key={session.id}
                    session={session}
                    agentName={agent?.name}
                    model={session.model_id ? modelMap.get(session.model_id) : undefined}
                    onTogglePin={handleTogglePin}
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
            <DialogDescription>
              Select a harness and optionally an agent to start a new conversation.
            </DialogDescription>
          </DialogHeader>
          <div className="py-4 space-y-4">
            <div className="space-y-2">
              <Label>Harness (required)</Label>
              <HarnessSelect
                value={newSessionHarnessId}
                onValueChange={setNewSessionHarnessId}
                placeholder="Select a harness"
              />
            </div>
            <div className="space-y-2">
              <Label>Agent (optional)</Label>
              <AgentSelect
                value={newSessionAgentId}
                onValueChange={setNewSessionAgentId}
                placeholder="Select an agent"
                includeAll
                allLabel="No agent"
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setNewSessionDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleCreateSession}
              disabled={!newSessionHarnessId || createSession.isPending}
            >
              {createSession.isPending ? "Creating..." : "Create Session"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
