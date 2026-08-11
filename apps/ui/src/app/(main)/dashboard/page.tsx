"use client";

import { useState } from "react";
import {
  useAgents,
  useCapabilities,
  useSessions,
  useSessionStats,
  useModels,
  useCreateSession,
  usePageTitle,
} from "@/hooks";
import { useRouter } from "next/navigation";
import { StatsCards } from "@/components/dashboard/stats-cards";
import { AgentListWidget, MAX_DISPLAYED_AGENTS } from "@/components/dashboard/agent-list-widget";
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
import { NewAgentLink } from "@/components/dashboard/new-agent-link";
import { Button } from "@/components/ui/button";
import { Plus, LayoutDashboard } from "lucide-react";
import { PageContainer, PageMasthead } from "@/components/layout";

export default function DashboardPage() {
  usePageTitle("Dashboard");
  const router = useRouter();
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: agentsForReferences = [] } = useAgents({ includeArchived: true });
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(undefined, {
    limit: 5,
  });
  const { data: sessionStats } = useSessionStats();

  const sessions = sessionsResponse?.data ?? [];

  // Defer the catalog fetches until the rendered dashboard state actually needs
  // them. On an empty org there are no capability-bearing agents and no
  // model-bearing sessions, so both large catalogs (~228 KB + ~78 KB) are pure
  // overfetch. The `enabled` gates flip on once the agent/session lists load
  // with content that references those catalogs. See EVE-783.
  const needsCapabilities = agents
    .filter((a) => a.status === "active")
    .slice(0, MAX_DISPLAYED_AGENTS)
    .some((a) => (a.capabilities?.length ?? 0) > 0);
  const needsModels = sessions.some((s) => !!s.model_id);

  const { data: allCapabilities } = useCapabilities({ enabled: needsCapabilities });
  const { data: models } = useModels({ enabled: needsModels });
  const createSession = useCreateSession();

  const [newSessionDialogOpen, setNewSessionDialogOpen] = useState(false);
  const [newSessionAgentId, setNewSessionAgentId] = useState<string>("");

  const handleOpenNewSessionDialog = () => {
    setNewSessionAgentId(agents[0]?.id || "");
    setNewSessionDialogOpen(true);
  };

  const handleCreateSession = async () => {
    if (!newSessionAgentId) return;
    try {
      // Agent-first: the server derives the harness from the selected agent.
      const session = await createSession.mutateAsync({
        request: {
          agent_id: newSessionAgentId,
        },
      });
      setNewSessionDialogOpen(false);
      router.push(`/sessions/${session.id}/transcript`);
    } catch (error) {
      console.error("Failed to create session:", error);
    }
  };

  const newSessionAction = (
    <Button variant="accent" onClick={handleOpenNewSessionDialog} disabled={agents.length === 0}>
      <Plus className="size-4" />
      New session
    </Button>
  );

  const dashboardMasthead = (
    <PageMasthead
      icon={<LayoutDashboard />}
      title="Dashboard"
      description="Your agents, recent sessions, and activity at a glance."
      actions={newSessionAction}
      compactActions={newSessionAction}
    />
  );

  if (agentsLoading || sessionsLoading) {
    return (
      <PageContainer>
        {dashboardMasthead}
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
          {[...Array(4)].map((_, i) => (
            <Skeleton key={i} className="h-32" />
          ))}
        </div>
        <div className="grid gap-6 md:grid-cols-2">
          <Skeleton className="h-96" />
          <Skeleton className="h-96" />
        </div>
      </PageContainer>
    );
  }

  return (
    <>
      <PageContainer>
        {dashboardMasthead}
        <StatsCards agents={agents} sessionStats={sessionStats} />
        <div className="grid gap-6 md:grid-cols-2">
          <AgentListWidget agents={agents} allCapabilities={allCapabilities} />

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
              <NewAgentLink className="block">
                <Button variant="outline" className="w-full justify-start">
                  <Plus className="h-4 w-4 mr-2" />
                  New Agent
                </Button>
              </NewAgentLink>
            </CardContent>
          </Card>
        </div>

        <RecentSessions sessions={sessions} agents={agentsForReferences} models={models} />
      </PageContainer>

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
    </>
  );
}
