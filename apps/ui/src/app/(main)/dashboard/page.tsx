"use client";

import { useAgents, useCapabilities, useAgentCapabilitiesBulk } from "@/hooks";
import { Header } from "@/components/layout/header";
import { StatsCards } from "@/components/dashboard/stats-cards";
import { AgentListWidget } from "@/components/dashboard/agent-list-widget";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus, Boxes } from "lucide-react";

export default function DashboardPage() {
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: allCapabilities } = useCapabilities();

  // Get agent IDs for bulk capabilities fetch
  const agentIds = agents?.map((a) => a.id) || [];
  const { data: agentCapabilitiesMap } = useAgentCapabilitiesBulk(agentIds);

  if (agentsLoading) {
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

  // For now, pass empty sessions array since we don't have a global sessions endpoint yet
  const sessions: [] = [];

  return (
    <>
      <Header title="Dashboard" />
      <div className="p-6 space-y-6">
        <StatsCards agents={agents} sessions={sessions} />
        <div className="grid gap-6 md:grid-cols-2">
          <AgentListWidget
            agents={agents}
            agentCapabilitiesMap={agentCapabilitiesMap}
            allCapabilities={allCapabilities}
          />

          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <CardTitle>Quick Actions</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <Link href="/agents/new" className="block">
                <Button variant="outline" className="w-full justify-start">
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
      </div>
    </>
  );
}
