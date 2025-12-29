"use client";

import { useAgents, useCapabilities } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Plus } from "lucide-react";
import { AgentCard } from "@/components/agents";

export default function AgentsPage() {
  const { data: agents, isLoading, error } = useAgents();
  const { data: allCapabilities } = useCapabilities();

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">
          Error loading agents: {error.message}
        </div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Agents</h1>
        <Link href="/agents/new">
          <Button>
            <Plus className="w-4 h-4 mr-2" />
            New Agent
          </Button>
        </Link>
      </div>

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {[...Array(6)].map((_, i) => (
            <Card key={i}>
              <CardHeader>
                <Skeleton className="h-6 w-3/4" />
              </CardHeader>
              <CardContent>
                <Skeleton className="h-4 w-full mb-2" />
                <Skeleton className="h-4 w-2/3" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : agents?.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-muted-foreground mb-4">No agents yet</p>
          <Link href="/agents/new">
            <Button>
              <Plus className="w-4 h-4 mr-2" />
              Create your first agent
            </Button>
          </Link>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {agents?.map((agent) => (
            <AgentCard
              key={agent.id}
              agent={agent}
              allCapabilities={allCapabilities}
              showEditButton
            />
          ))}
        </div>
      )}
    </div>
  );
}
