"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Boxes, MessageSquare, CheckCircle, Clock } from "lucide-react";
import type { Agent, Session } from "@/lib/api/types";

interface StatsCardsProps {
  agents: Agent[];
  sessions: Session[];
}

export function StatsCards({ agents, sessions }: StatsCardsProps) {
  const activeAgents = agents.filter((a) => a.status === "active").length;
  const runningSessions = sessions.filter((s) => s.status === "running").length;
  const completedSessions = sessions.filter((s) => s.status === "completed").length;
  const totalSessions = sessions.length;
  const completionRate =
    totalSessions > 0 ? Math.round((completedSessions / totalSessions) * 100) : 0;

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
      title: "Completion Rate",
      value: `${completionRate}%`,
      description: `${completedSessions} completed`,
      icon: CheckCircle,
      color: "text-green-600",
    },
    {
      title: "Pending",
      value: sessions.filter((s) => s.status === "pending").length,
      description: "Not yet started",
      icon: Clock,
      color: "text-gray-600",
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
