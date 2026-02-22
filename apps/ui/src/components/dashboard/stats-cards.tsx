"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Boxes, MessageSquare, CheckCircle, Clock, Hash } from "lucide-react";
import type { Agent, SessionStats } from "@/lib/api/types";

interface StatsCardsProps {
  agents: Agent[];
  sessionStats?: SessionStats;
}

export function StatsCards({ agents, sessionStats }: StatsCardsProps) {
  const activeAgents = agents.filter((a) => a.status === "active").length;

  const stats = [
    {
      title: "Total Agents",
      value: agents.length,
      description: `${activeAgents} active`,
      icon: Boxes,
      color: "text-blue-600",
    },
    {
      title: "Total Sessions",
      value: sessionStats?.total ?? 0,
      description: "All sessions",
      icon: Hash,
      color: "text-purple-600",
    },
    {
      title: "Active Sessions",
      value: sessionStats?.active ?? 0,
      description: "Currently processing",
      icon: MessageSquare,
      color: "text-yellow-600",
    },
    {
      title: "Idle Sessions",
      value: sessionStats?.idle ?? 0,
      description: "Ready for input",
      icon: CheckCircle,
      color: "text-green-600",
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
