"use client";

import { useState } from "react";
import {
  useAgents,
  useHarnesses,
  useCapabilities,
  useSessions,
  useSessionStats,
  useModels,
  useCreateSession,
  usePageTitle,
} from "@/hooks";
import { useRouter } from "next/navigation";
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
import { Plus, LayoutDashboard } from "lucide-react";
import { useOrganization } from "@/hooks/use-organizations";
import { PageContainer, PageMasthead } from "@/components/layout";

export default function DashboardPage() {
  usePageTitle("Dashboard");
  const router = useRouter();
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: agentsForReferences = [] } = useAgents({ includeArchived: true });
  const { data: allCapabilities } = useCapabilities();
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(undefined, {
    limit: 5,
  });
  const { data: sessionStats } = useSessionStats();
  const { data: models } = useModels();
  const createSession = useCreateSession();
  const { data: organization } = useOrganization();
  const { data: harnesses } = useHarnesses();

  const [newSessionDialogOpen, setNewSessionDialogOpen] = useState(false);
  const [newSessionAgentId, setNewSessionAgentId] = useState<string>("");

  const handleOpenNewSessionDialog = () => {
    setNewSessionAgentId(agents[0]?.id || "");
    setNewSessionDialogOpen(true);
  };

  const handleCreateSession = async () => {
    const harnessId = organization?.default_harness_id || harnesses?.[0]?.id;
    if (!harnessId) return;
    try {
      const session = await createSession.mutateAsync({
        request: {
          harness_id: harnessId,
          agent_id: newSessionAgentId || undefined,
        },
      });
      setNewSessionDialogOpen(false);
      router.push(`/sessions/${session.id}`);
    } catch (error) {
      console.error("Failed to create session:", error);
    }
  };

  const dashboardMasthead = (
    <PageMasthead
      icon={<LayoutDashboard />}
      title="Dashboard"
      description="Your agents, recent sessions, and activity at a glance."
      actions={
        <Button
          variant="accent"
          onClick={handleOpenNewSessionDialog}
          disabled={agents.length === 0}
        >
          <Plus className="size-4" />
          New session
        </Button>
      }
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

  const sessions = sessionsResponse?.data ?? [];

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
              <Link href="/agents/new" className="block">
                <Button variant="outline" className="w-full justify-start">
                  <Plus className="h-4 w-4 mr-2" />
                  New Agent
                </Button>
              </Link>
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
