"use client";

import { useMemo } from "react";
import { useAgents, useCapabilities, useSessions, useLlmModels } from "@/hooks";
import { Header } from "@/components/layout/header";
import { StatsCards } from "@/components/dashboard/stats-cards";
import { AgentListWidget } from "@/components/dashboard/agent-list-widget";
import { RecentSessions } from "@/components/dashboard/recent-sessions";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus, Boxes } from "lucide-react";
import type { Agent, LlmModelWithProvider } from "@/lib/api/types";

export default function DashboardPage() {
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: allCapabilities } = useCapabilities();
  const { data: sessionsResponse, isLoading: sessionsLoading } = useSessions(undefined, { limit: 5 });
  const { data: llmModels } = useLlmModels();

  // Create a map of agent_id -> agent for quick lookups
  const agentMap = useMemo(() => {
    return new Map<string, Agent>(agents.map((a) => [a.id, a]));
  }, [agents]);

  // Create a map of model_id -> model for quick lookups
  const modelMap = useMemo(() => {
    if (!llmModels) return new Map<string, LlmModelWithProvider>();
    return new Map(llmModels.map((m) => [m.id, m]));
  }, [llmModels]);

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
              <Link href="/agents/new" className="block">
                <Button variant="accent" className="w-full justify-start">
                  <Plus className="h-4 w-4 mr-2" />
                  Create New Agent
                </Button>
              </Link>
              <Link href="/agents" className="block">
                <Button variant="outline" className="w-full justify-start">
                  <Boxes className="h-4 w-4 mr-2" />
                  Browse All Agents
                </Button>
              </Link>
              {agents.length > 0 && (
                <p className="text-sm text-muted-foreground">
                  Select an agent to view its sessions and start conversations.
                </p>
              )}
            </CardContent>
          </Card>
        </div>

        <RecentSessions
          sessions={sessions}
          agents={agents}
          models={llmModels}
        />
      </div>
    </>
  );
}
