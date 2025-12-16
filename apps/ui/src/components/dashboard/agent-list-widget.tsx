"use client";

import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Boxes, Plus } from "lucide-react";
import type { Agent } from "@/lib/api/types";

interface AgentListWidgetProps {
  agents: Agent[];
}

export function AgentListWidget({ agents }: AgentListWidgetProps) {
  const activeAgents = agents.filter((a) => a.status === "active").slice(0, 5);

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>Active Agents</CardTitle>
        <Link href="/agents/new">
          <Button variant="outline" size="sm">
            <Plus className="h-4 w-4 mr-1" />
            New Agent
          </Button>
        </Link>
      </CardHeader>
      <CardContent>
        {activeAgents.length === 0 ? (
          <div className="text-center py-8">
            <Boxes className="h-12 w-12 mx-auto text-muted-foreground mb-2" />
            <p className="text-muted-foreground">No agents yet.</p>
            <Link href="/agents/new">
              <Button variant="link">Create your first agent</Button>
            </Link>
          </div>
        ) : (
          <div className="space-y-3">
            {activeAgents.map((agent) => (
              <Link
                key={agent.id}
                href={`/agents/${agent.id}`}
                className="flex items-center justify-between p-3 rounded-lg border hover:bg-accent transition-colors"
              >
                <div className="flex items-center gap-3">
                  <Boxes className="h-5 w-5 text-muted-foreground" />
                  <div>
                    <p className="font-medium">{agent.name}</p>
                    <p className="text-xs text-muted-foreground font-mono">
                      {agent.id.slice(0, 8)}...
                    </p>
                  </div>
                </div>
                <Badge
                  variant="outline"
                  className="bg-green-100 text-green-800"
                >
                  Active
                </Badge>
              </Link>
            ))}
            {agents.length > 5 && (
              <Link href="/agents">
                <Button variant="ghost" className="w-full">
                  View all {agents.length} agents
                </Button>
              </Link>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
