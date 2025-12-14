"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Bot, Play, CheckCircle, XCircle } from "lucide-react";
import type { Agent, Run } from "@/lib/api/types";

interface StatsCardsProps {
  agents: Agent[];
  runs: Run[];
}

export function StatsCards({ agents, runs }: StatsCardsProps) {
  const activeAgents = agents.filter((a) => a.status === "active").length;
  const activeRuns = runs.filter(
    (r) => r.status === "pending" || r.status === "running"
  ).length;
  const completedRuns = runs.filter((r) => r.status === "completed").length;
  const failedRuns = runs.filter((r) => r.status === "failed").length;
  const totalRuns = runs.length;
  const successRate =
    totalRuns > 0 ? Math.round((completedRuns / totalRuns) * 100) : 0;

  const stats = [
    {
      title: "Total Agents",
      value: agents.length,
      description: `${activeAgents} active`,
      icon: Bot,
      color: "text-blue-600",
    },
    {
      title: "Active Runs",
      value: activeRuns,
      description: "Currently executing",
      icon: Play,
      color: "text-yellow-600",
    },
    {
      title: "Success Rate",
      value: `${successRate}%`,
      description: `${completedRuns} completed`,
      icon: CheckCircle,
      color: "text-green-600",
    },
    {
      title: "Failed Runs",
      value: failedRuns,
      description: "Needs attention",
      icon: XCircle,
      color: "text-red-600",
    },
  ];

  return (
    <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
      {stats.map((stat) => (
        <Card key={stat.title}>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">{stat.title}</CardTitle>
            <stat.icon className={`h-4 w-4 ${stat.color}`} />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold">{stat.value}</div>
            <p className="text-xs text-muted-foreground">{stat.description}</p>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
