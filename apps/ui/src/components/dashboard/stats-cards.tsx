"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Boxes, MessageSquare, CheckCircle, Clock } from "lucide-react";
import type { Agent, Session } from "@/lib/api/types";

interface StatsCardsProps {
  agents: Agent[];
  sessions: Session[];
}

// Session status: started → active → idle (cycles)
// - started: Session just created, no turn executed yet
// - active: A turn is currently running
// - idle: Turn completed, session waiting for next input
export function StatsCards({ agents, sessions }: StatsCardsProps) {
  const activeAgents = agents.filter((a) => a.status === "active").length;
  const activeSessions = sessions.filter((s) => s.status === "active").length;
  const idleSessions = sessions.filter((s) => s.status === "idle").length;
  const newSessions = sessions.filter((s) => s.status === "started").length;

  const stats = [
    {
      title: "Total Agents",
      value: agents.length,
      description: `${activeAgents} active`,
      icon: Boxes,
      color: "text-blue-600",
    },
    {
      title: "Active Sessions",
      value: activeSessions,
      description: "Currently processing",
      icon: MessageSquare,
      color: "text-yellow-600",
    },
    {
      title: "Idle Sessions",
      value: idleSessions,
      description: "Ready for input",
      icon: CheckCircle,
      color: "text-green-600",
    },
    {
      title: "New Sessions",
      value: newSessions,
      description: "Just created",
      icon: Clock,
      color: "text-blue-400",
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
