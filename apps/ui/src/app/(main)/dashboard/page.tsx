"use client";

import { useMemo, useState } from "react";
import { useAgents, useCapabilities, useSessions, useLlmModels, useCreateSession } from "@/hooks";
import { useRouter } from "next/navigation";
import { Header } from "@/components/layout/header";
import { StatsCards } from "@/components/dashboard/stats-cards";
import { AgentListWidget } from "@/components/dashboard/agent-list-widget";
import { RecentSessions } from "@/components/dashboard/recent-sessions";
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
import { AgentSelect } from "@/components/agent/agent-select";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus } from "lucide-react";
import type { Agent, LlmModelWithProvider } from "@/lib/api/types";

export default function DashboardPage() {
  const router = useRouter();
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: allCapabilities } = useCapabilities();
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(undefined, { limit: 5 });
  const { data: llmModels } = useLlmModels();
  const createSession = useCreateSession();

  const [newSessionDialogOpen, setNewSessionDialogOpen] = useState(false);
  const [newSessionAgentId, setNewSessionAgentId] = useState<string>("");

  // Create a map of agent_id -> agent for quick lookups
  const agentMap = useMemo(() => {
    return new Map<string, Agent>(agents.map((a) => [a.id, a]));
  }, [agents]);

  // Create a map of model_id -> model for quick lookups
  const modelMap = useMemo(() => {
    if (!llmModels) return new Map<string, LlmModelWithProvider>();
    return new Map(llmModels.map((m) => [m.id, m]));
  }, [llmModels]);

  const handleOpenNewSessionDialog = () => {
    setNewSessionAgentId(agents[0]?.id || "");
    setNewSessionDialogOpen(true);
  };

  const handleCreateSession = async () => {
    if (!newSessionAgentId) return;
    try {
      const session = await createSession.mutateAsync({
        request: { agent_id: newSessionAgentId },
      });
      setNewSessionDialogOpen(false);
      router.push(`/sessions/${session.id}`);
    } catch (error) {
      console.error("Failed to create session:", error);
    }
  };

  if (agentsLoading || sessionsLoading) {
    return (
      <>
        <Header title="Dashboard" />
        <div className="p-6 space-y-6">
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            {[...Array(4)].map((_, i) => (
              <Skeleton key={i} className="h-32" />
            ))}
          </div>
          <div className="grid gap-6 md:grid-cols-2">
            <Skeleton className="h-96" />
            <Skeleton className="h-96" />
          </div>
        </div>
      </>
    );
  }

  const sessions = sessionsResponse?.data ?? [];

  return (
    <>
      <Header title="Dashboard" />
      <div className="p-6 space-y-6">
        <StatsCards agents={agents} sessions={sessions} />
        <div className="grid gap-6 md:grid-cols-2">
          <AgentListWidget
            agents={agents}
            allCapabilities={allCapabilities}
          />

          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <CardTitle>Quick Actions</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              {agents.length > 0 && (
                <Button
                  variant="accent"
                  className="w-full justify-start"
                  onClick={handleOpenNewSessionDialog}
                >
                  <Plus className="h-4 w-4 mr-2" />
                  New Session
                </Button>
              )}
              <Link href="/agents/new" className="block">
                <Button variant="outline" className="w-full justify-start">
                  <Plus className="h-4 w-4 mr-2" />
                  New Agent
                </Button>
              </Link>
            </CardContent>
          </Card>
        </div>

        <RecentSessions
          sessions={sessions}
          agents={agents}
          models={llmModels}
        />
      </div>

      {/* New Session Dialog */}
      <Dialog open={newSessionDialogOpen} onOpenChange={setNewSessionDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New Session</DialogTitle>
            <DialogDescription>
              Select an agent to start a new conversation.
            </DialogDescription>
          </DialogHeader>
          <div className="py-4">
            <AgentSelect
              value={newSessionAgentId}
              onValueChange={setNewSessionAgentId}
              placeholder="Select an agent"
            />
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setNewSessionDialogOpen(false)}
            >
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
    </>
  );
}
