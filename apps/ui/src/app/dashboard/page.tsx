"use client";

import { useAgents } from "@/hooks/use-agents";
import { useRuns } from "@/hooks/use-runs";
import { Header } from "@/components/layout/header";
import { StatsCards } from "@/components/dashboard/stats-cards";
import { RecentRuns } from "@/components/dashboard/recent-runs";
import { AgentListWidget } from "@/components/dashboard/agent-list-widget";
import { Skeleton } from "@/components/ui/skeleton";

export default function DashboardPage() {
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: runs = [], isLoading: runsLoading } = useRuns();

  const isLoading = agentsLoading || runsLoading;

  if (isLoading) {
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

  return (
    <>
      <Header title="Dashboard" />
      <div className="p-6 space-y-6">
        <StatsCards agents={agents} runs={runs} />
        <div className="grid gap-6 md:grid-cols-2">
          <RecentRuns runs={runs} agents={agents} />
          <AgentListWidget agents={agents} />
        </div>
      </div>
    </>
  );
}
