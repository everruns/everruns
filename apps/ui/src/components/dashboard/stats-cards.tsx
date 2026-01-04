"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Boxes, MessageSquare, CheckCircle, Clock } from "lucide-react";
import type { Agent, Session } from "@/lib/api/types";

interface StatsCardsProps {
  agents: Agent[];
  sessions: Session[];
}

// Session status: pending → running → pending (cycles) | failed
// Sessions return to "pending" after processing - there is no "completed" state
export function StatsCards({ agents, sessions }: StatsCardsProps) {
  const activeAgents = agents.filter((a) => a.status === "active").length;
  const runningSessions = sessions.filter((s) => s.status === "running").length;
  const pendingSessions = sessions.filter((s) => s.status === "pending").length;
  const failedSessions = sessions.filter((s) => s.status === "failed").length;

  const stats = [
    {
      title: "Total Agents",
      value: agents.length,
      description: `${activeAgents} active`,
      icon: Boxes,
      color: "text-blue-600",
    },
    {
      title: "Running Sessions",
      value: runningSessions,
      description: "Currently running",
      icon: MessageSquare,
      color: "text-yellow-600",
    },
    {
      title: "Pending Sessions",
      value: pendingSessions,
      description: "Ready for input",
      icon: CheckCircle,
      color: "text-green-600",
    },
    {
      title: "Failed Sessions",
      value: failedSessions,
      description: "Terminated with error",
      icon: Clock,
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
